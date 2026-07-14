use bitcoin::address::NetworkUnchecked;
use bitcoin::consensus;
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::hashes::Hash;
use bitcoin::{
    absolute, transaction, Address, Amount, BlockHash, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness,
};
use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_reorg_metrics, assert_wallet_metrics, create_client,
    get_all_nodes, random_node, random_range, Client,
};
use serde_json::json;
use std::collections::HashSet;

const MAX_INPUTS: usize = 20;
const MAX_OUTPUTS: usize = 20;

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct Utxo {
    txid: Txid,
    vout: u32,
    #[serde(with = "bitcoin::amount::serde::as_btc")]
    amount: Amount,
    #[serde(rename = "scriptPubKey")]
    script_pubkey: ScriptBuf,
}

impl Utxo {
    fn out_point(&self) -> OutPoint {
        OutPoint {
            txid: self.txid,
            vout: self.vout,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Corruption {
    None,
    MissingInput,
    BadSignature,
    DupInputInTx,
    DupInputInBlock,
}

impl Corruption {
    fn random() -> Self {
        match random_range(5) {
            0 => Self::MissingInput,
            1 => Self::BadSignature,
            2 => Self::DupInputInTx,
            3 => Self::DupInputInBlock,
            _ => Self::None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MissingInput => "missing-input",
            Self::BadSignature => "bad-sig",
            Self::DupInputInTx => "dup-input-in-tx",
            Self::DupInputInBlock => "dup-input-in-block",
        }
    }

    fn build(self, client: &Client, pool: &mut BlockUtxoSet) -> Option<Transaction> {
        match self {
            Self::None => build_general_tx(client, pool),
            Self::MissingInput => build_missing_input_tx(client, pool),
            Self::DupInputInTx => build_dup_input_in_tx(client, pool),
            Self::DupInputInBlock => build_dup_input_in_block(client, pool),
            Self::BadSignature => build_bad_signature_tx(client, pool),
        }
    }
}

struct BlockUtxoSet {
    available: Vec<Utxo>,
    // UTXOs spent by txs that were actually added to the block. `spent` must
    // never contain UTXOs whose spending tx failed to build: DupInputInBlock
    // picks its conflict from here, and a "conflict" nothing spends would make
    // the corrupted block valid, falsely tripping the always-assertion.
    spent: Vec<Utxo>,
    // UTXOs taken for the tx currently being built; moved to `spent` on
    // success or back to `available` on failure.
    pending: Vec<Utxo>,
    intra_block_txids: HashSet<Txid>,
}

impl BlockUtxoSet {
    fn new(available: Vec<Utxo>) -> Self {
        Self {
            available,
            spent: Vec::new(),
            pending: Vec::new(),
            intra_block_txids: HashSet::new(),
        }
    }

    /// Take up to `n` UTXOs out of `available` at random and record them as pending.
    fn take(&mut self, n: usize) -> Vec<Utxo> {
        let mut picked = Vec::with_capacity(n);
        for _ in 0..n {
            if self.available.is_empty() {
                break;
            }
            let i = random_range(self.available.len() as u64) as usize;
            let utxo = self.available.swap_remove(i);
            self.pending.push(utxo.clone());
            picked.push(utxo);
        }
        picked
    }

    /// The pending tx was added to the block; its inputs are now truly spent.
    fn commit_pending(&mut self) {
        self.spent.append(&mut self.pending);
    }

    /// The pending tx failed to build; its inputs are spendable again.
    fn rollback_pending(&mut self) {
        self.available.append(&mut self.pending);
    }

    /// Pick a UTXO some other raw tx in this block already claimed.
    fn random_spent(&self) -> Option<Utxo> {
        if self.spent.is_empty() {
            return None;
        }
        let i = random_range(self.spent.len() as u64) as usize;
        Some(self.spent[i].clone())
    }

    fn add_outputs(&mut self, tx: &Transaction) {
        let txid = tx.compute_txid();
        self.intra_block_txids.insert(txid);
        for (vout, out) in tx.output.iter().enumerate() {
            self.available.push(Utxo {
                txid,
                vout: vout as u32,
                amount: out.value,
                script_pubkey: out.script_pubkey.clone(),
            });
        }
    }

    fn has_intra_block_parent(&self, tx: &Transaction) -> bool {
        tx.input
            .iter()
            .any(|i| self.intra_block_txids.contains(&i.previous_output.txid))
    }
}

// ---------- Tx construction ----------

fn build_tx_from_inputs(client: &Client, inputs: &[Utxo]) -> Option<Transaction> {
    let total: Amount = inputs.iter().map(|u| u.amount).sum();
    let num_outputs = 1 + random_range(MAX_OUTPUTS as u64) as usize;
    let fee = Amount::from_sat(10_000 + 500 * num_outputs as u64);
    let amount_per_output = total.checked_sub(fee).unwrap_or(Amount::ZERO) / num_outputs as u64;

    let input: Vec<TxIn> = inputs
        .iter()
        .map(|u| TxIn {
            previous_output: u.out_point(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        })
        .collect();

    let mut output: Vec<TxOut> = Vec::with_capacity(num_outputs);
    for _ in 0..num_outputs {
        let address: Address = match client.call::<Address<NetworkUnchecked>>("getnewaddress", &[])
        {
            Ok(a) => a.assume_checked(),
            Err(e) => {
                eprintln!("[invalid-blocks] getnewaddress failed: {e}");
                return None;
            }
        };
        output.push(TxOut {
            value: amount_per_output,
            script_pubkey: address.script_pubkey(),
        });
    }

    let unsigned = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input,
        output,
    };

    let prevtxs: Vec<&Utxo> = inputs
        .iter()
        .filter(|u| !u.script_pubkey.is_empty())
        .collect();

    #[derive(serde::Deserialize)]
    struct SignResult {
        #[serde(with = "consensus::serde::With::<consensus::serde::Hex>")]
        hex: Transaction,
    }
    let signed: SignResult = match client.call(
        "signrawtransactionwithwallet",
        &[json!(serialize_hex(&unsigned)), json!(prevtxs)],
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[invalid-blocks] signrawtransactionwithwallet failed: {e}");
            return None;
        }
    };
    Some(signed.hex)
}

/// Build a "general" tx: 1..=`MAX_INPUTS` inputs and 1..=`MAX_OUTPUTS` outputs.
fn build_general_tx(client: &Client, pool: &mut BlockUtxoSet) -> Option<Transaction> {
    let n_inputs = 1 + random_range(MAX_INPUTS as u64) as usize;
    let inputs = pool.take(n_inputs);
    if inputs.is_empty() {
        return None;
    }
    build_tx_from_inputs(client, &inputs)
}

// ---------- Corruption builders ----------

/// Build a tx with an input that is not in the utxo set
fn build_missing_input_tx(client: &Client, pool: &mut BlockUtxoSet) -> Option<Transaction> {
    let missing_bytes: [u8; 32] = std::array::from_fn(|_| random_range(256) as u8);
    let missing = Utxo {
        txid: Txid::from_byte_array(missing_bytes),
        vout: random_range(8) as u32,
        amount: Amount::from_sat(100_000),
        script_pubkey: ScriptBuf::new(),
    };
    let n_extra = random_range(MAX_INPUTS as u64) as usize;
    let mut inputs = pool.take(n_extra);
    inputs.push(missing);
    build_tx_from_inputs(client, &inputs)
}

/// Build a tx that has the same input twice
fn build_dup_input_in_tx(client: &Client, pool: &mut BlockUtxoSet) -> Option<Transaction> {
    let dup = pool.take(1).into_iter().next()?;
    let n_extra = random_range(MAX_INPUTS as u64 - 1) as usize;
    let mut inputs = pool.take(n_extra);
    inputs.push(dup.clone());
    inputs.push(dup);
    build_tx_from_inputs(client, &inputs)
}

/// Build a tx that spends a UTXO already spent by another tx in this block
fn build_dup_input_in_block(client: &Client, pool: &mut BlockUtxoSet) -> Option<Transaction> {
    let conflict = pool.random_spent()?;
    let n_extra = random_range(MAX_INPUTS as u64) as usize;
    let mut inputs = pool.take(n_extra);
    inputs.push(conflict);
    build_tx_from_inputs(client, &inputs)
}

/// Build a regular tx and bit-flip one byte of a random input's witness or script sig
fn build_bad_signature_tx(client: &Client, pool: &mut BlockUtxoSet) -> Option<Transaction> {
    let mut tx = build_general_tx(client, pool)?;
    let i = random_range(tx.input.len() as u64) as usize;
    let input = &mut tx.input[i];

    if !input.witness.is_empty() {
        let mut stack: Vec<Vec<u8>> = input.witness.iter().map(<[u8]>::to_vec).collect();
        // A wallet-signed witness should have a non-empty stack item; if not, skip this
        // attempt rather than panicking.
        let item = stack.iter_mut().find(|i| !i.is_empty())?;
        let idx = random_range(item.len() as u64) as usize;
        item[idx] ^= 0xff;
        input.witness = Witness::from_slice(&stack);
    } else if !input.script_sig.is_empty() {
        let mut bytes = input.script_sig.as_bytes().to_vec();
        let idx = random_range(bytes.len() as u64) as usize;
        bytes[idx] ^= 0xff;
        input.script_sig = ScriptBuf::from_bytes(bytes);
    }

    Some(tx)
}

// ---------- Mempool sampling ----------

#[derive(Default)]
struct BlockMempoolTxs {
    txids: Vec<Txid>,
    consumed_utxos: HashSet<OutPoint>,
}

/// Take a prefix of the mempool
fn pick_mempool_block_txs(client: &Client, max: usize) -> BlockMempoolTxs {
    #[derive(serde::Deserialize)]
    struct TemplateTx {
        txid: Txid,
        #[serde(with = "consensus::serde::With::<consensus::serde::Hex>")]
        data: Transaction,
    }
    #[derive(serde::Deserialize)]
    struct BlockTemplate {
        transactions: Vec<TemplateTx>,
    }

    #[derive(serde::Serialize)]
    struct GbtRequest {
        rules: &'static [&'static str],
    }
    let req = GbtRequest { rules: &["segwit"] };
    let template: BlockTemplate = match client.call("getblocktemplate", &[json!(req)]) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[invalid-blocks] getblocktemplate failed: {e}");
            return BlockMempoolTxs::default();
        }
    };

    let take = (random_range(max as u64 + 1) as usize).min(template.transactions.len());
    let mut txids = Vec::with_capacity(take);
    let mut consumed_utxos = HashSet::new();

    for entry in template.transactions.into_iter().take(take) {
        txids.push(entry.txid);
        for input in &entry.data.input {
            consumed_utxos.insert(input.previous_output);
        }
    }

    BlockMempoolTxs {
        txids,
        consumed_utxos,
    }
}

// ---------- main ----------

fn main() {
    let nodes = get_all_nodes();

    // Pick a random node to mine on
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[invalid-blocks] Failed to create client: {e}");
            return;
        }
    };

    let num_blocks = 1 + random_range(3);

    for block_num in 0..num_blocks {
        let addr: Address = match client.call::<Address<NetworkUnchecked>>("getnewaddress", &[]) {
            Ok(a) => a.assume_checked(),
            Err(e) => {
                eprintln!("[invalid-blocks] getnewaddress failed: {e}");
                return;
            }
        };

        // Pick a topologically-ordered prefix of the mempool
        let mempool_block = pick_mempool_block_txs(&client, 32);
        let mempool_count = mempool_block.txids.len();
        let utxos: Vec<Utxo> = client
            .call("listunspent", &[json!(1), json!(9_999_999)])
            .ok()
            .unwrap_or_default();
        let available = utxos
            .into_iter()
            .filter(|u| !mempool_block.consumed_utxos.contains(&u.out_point()))
            .collect();
        let mut pool = BlockUtxoSet::new(available);

        // `txs` starts as the mempool prefix (as txids) and we splice raw txs into it as we
        // build them. Mempool entries stay in place relative to one another, so the
        // parents-before-children ordering is preserved
        let mut txs: Vec<String> = mempool_block.txids.iter().map(Txid::to_string).collect();

        // 1..=32 raw txs total, with at most one of them being the corruption candidate.
        let n_total = 1 + random_range(32) as usize;
        let corrupt_at = random_range(n_total as u64) as usize;
        let candidate = Corruption::random();
        let mut applied = Corruption::None;

        for i in 0..n_total {
            let kind = if i == corrupt_at {
                candidate
            } else {
                Corruption::None
            };
            let Some(tx) = kind.build(&client, &mut pool) else {
                pool.rollback_pending();
                continue;
            };
            pool.commit_pending();
            // If this tx spends an output of an earlier raw tx in this block, the parent
            // must come first in the block's tx vector. Append at the end so we're
            // guaranteed to be after the parent
            let pos = if pool.has_intra_block_parent(&tx) {
                txs.len()
            } else {
                random_range(txs.len() as u64 + 1) as usize
            };
            txs.insert(pos, serialize_hex(&tx));
            // Make this tx's outputs available to subsequent raw txs in the same block.
            pool.add_outputs(&tx);
            if kind != Corruption::None {
                applied = kind;
            }
        }

        #[derive(serde::Deserialize)]
        struct GenerateBlockResult {
            hash: BlockHash,
        }
        let result: Result<GenerateBlockResult, _> =
            client.call("generateblock", &[json!(addr), json!(txs)]);
        let accepted = result.is_ok();
        let err_msg = result.as_ref().err().map(|e| format!("{e}"));
        let hash = result.as_ref().ok().map(|r| r.hash);

        let raw_count = txs.len() - mempool_count;
        let corrupted = applied != Corruption::None;

        #[derive(serde::Serialize)]
        struct BlockDetails<'a> {
            block_num: u64,
            node: &'a str,
            address: &'a Address,
            corruption: &'a str,
            mempool_txs: usize,
            raw_txs: usize,
            accepted: bool,
            block_hash: Option<BlockHash>,
            error: Option<&'a str>,
        }
        let details = serde_json::to_value(BlockDetails {
            block_num: block_num + 1,
            node: &node_config.host,
            address: &addr,
            corruption: applied.name(),
            mempool_txs: mempool_count,
            raw_txs: raw_count,
            accepted,
            block_hash: hash,
            error: err_msg.as_deref(),
        })
        .expect("BlockDetails serializes");

        // INVARIANT: A corrupted block must NEVER be accepted.
        antithesis_sdk::assert_always!(
            !(corrupted && accepted),
            "Block with deliberate consensus violation must be rejected",
            &details
        );

        // Coverage targets - we want to see each of these states at least once.
        antithesis_sdk::assert_sometimes!(
            !corrupted && accepted,
            "Valid generateblock submission was accepted",
            &details
        );
        antithesis_sdk::assert_sometimes!(
            applied == Corruption::MissingInput && !accepted,
            "Block with a missing-input tx was rejected",
            &details
        );
        antithesis_sdk::assert_sometimes!(
            applied == Corruption::BadSignature && !accepted,
            "Block with a corrupted-signature tx was rejected",
            &details
        );
        antithesis_sdk::assert_sometimes!(
            applied == Corruption::DupInputInTx && !accepted,
            "Block with a duplicate-input tx was rejected",
            &details
        );
        antithesis_sdk::assert_sometimes!(
            applied == Corruption::DupInputInBlock && !accepted,
            "Block with a within-block double-spend was rejected",
            &details
        );

        let outcome = match &hash {
            Some(h) => format!("accepted: {h}"),
            None => format!("rejected: {}", err_msg.as_deref().unwrap_or("")),
        };
        let line = format!(
            "[invalid-blocks] generateblock #{} to {} (mempool_txs={}, raw_txs={}, corruption={}) -> {}",
            block_num + 1,
            addr,
            mempool_count,
            raw_count,
            applied.name(),
            outcome
        );
        if accepted {
            println!("{line}");
        } else {
            eprintln!("{line}");
        }

        assert_reorg_metrics(&client, "after_generateblock");
        assert_mempool_metrics(&client, "after_generateblock");
        assert_wallet_metrics(&client, "after_generateblock");
    }
}
