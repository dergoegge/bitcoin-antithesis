pub mod mining;

use bitcoin::{
    Amount, OutPoint, Sequence, Transaction, TxIn, TxOut, Witness,
    blockdata::opcodes::{OP_0, OP_TRUE},
    script::ScriptBuf,
    transaction,
};

use bitcoin_hashes::sha256;

/// Create a consolidation transaction at 1 sat/vb that consolidates all provided inputs into a
/// single output.
///
/// Expects the inputs to be P2WSH-OP_TRUE outputs (i.e. OP_0 sha256([OP_TRUE])) and the created
/// output will also be a P2WSH-OP_TRUE output.
pub fn create_consolidation_tx(
    inputs: &[(OutPoint, bitcoin::Amount)],
) -> Result<Transaction, String> {
    let feerate = bitcoin::FeeRate::from_sat_per_vb(1).unwrap();

    let mut p2wsh_optrue_spk = vec![OP_0.to_u8(), 32];
    let op_true_hash = sha256::Hash::hash(&[OP_TRUE.to_u8()]);
    p2wsh_optrue_spk.extend(op_true_hash.as_byte_array().as_slice());

    let mut p2wsh_optrue_witness = Witness::new();
    p2wsh_optrue_witness.push([OP_TRUE.to_u8()]);

    let input = inputs
        .iter()
        .map(|(outpoint, _)| TxIn {
            previous_output: *outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence(0xFFFFFFFF),
            witness: p2wsh_optrue_witness.clone(),
        })
        .collect();

    let total_input_value: Amount = inputs.iter().map(|(_, amount)| *amount).sum();

    // Create a transaction to estimate its size
    let mut tx = Transaction {
        version: transaction::Version(1),
        lock_time: bitcoin::absolute::LockTime::from_height(0).unwrap(),
        input,
        output: vec![TxOut {
            value: total_input_value,
            script_pubkey: p2wsh_optrue_spk.into(),
        }],
    };

    // Calculate fee based on the virtual size
    let fee = feerate.fee_wu(tx.weight()).unwrap();

    // Ensure we have enough funds to pay the fee
    if total_input_value < fee {
        return Err("Insufficient funds to pay for transaction fee".to_string());
    }

    tx.output[0].value = total_input_value - fee;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{BlockHash, hashes::Hash};

    #[test]
    fn test_commitment_fixup() {
        let mut block = mining::mine_block(BlockHash::all_zeros(), 1, 2).unwrap();
        block.txdata.push(
            create_consolidation_tx(&[
                (OutPoint::null(), Amount::from_int_btc(10)),
                (OutPoint::null(), Amount::from_int_btc(20)),
            ])
            .unwrap(),
        );
        mining::fixup_commitments(&mut block);
        assert!(block.check_merkle_root());
        assert!(block.check_witness_commitment());
    }
}
