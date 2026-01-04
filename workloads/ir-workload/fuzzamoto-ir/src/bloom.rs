/// Rust transplation from https://github.com/bitcoin/bitcoin/blob/master/src/common/bloom.cpp
// why do we need this code? because `filterload` message require us that we send the filter itself. So we need to construct the bloom filter on the client (fuzzer) side too.
use murmurs::murmur3_x86_32;

// https://github.com/bitcoin/bips/blob/master/bip-0037.mediawiki?plain=1#L51
/// Maximum size for filterload filter
pub(crate) const MAX_BLOOM_FILTER_SIZE: u32 = 36000;
pub(crate) const MAX_HASH_FUNCS: u32 = 50;

// Hash the data
pub fn hash(hashnum: u32, size: usize, key: &[u8]) -> u32 {
    // this needs to be wrapping else it panics in debug mode
    let a = murmur3_x86_32(key, hashnum.wrapping_mul(0xFBA4C795));
    let b = (size * 8) as u32;
    a % b
}

pub fn filter_insert(data: &mut [u8], n_hash_funcs: u32, key: &[u8]) {
    if data.is_empty() {
        return;
    }
    for i in 0..n_hash_funcs {
        let index = hash(i, data.len(), key);
        let index_into_data = (index >> 3) as usize;
        let bits_to_raise = 7 & index;
        data[index_into_data] |= 1 << bits_to_raise;
    }
}
