#![no_main]

sp1_zkvm::entrypoint!(main);

use sp1_zkvm::lib::bls12381::decompress_pubkey;

pub fn main() {
    let compressed_key = sp1_zkvm::io::read::<[u64; 6]>();

    for _ in 0..4 {
        println!("before: {:?}", compressed_key);

        let decompressed_key = decompress_pubkey(&compressed_key).unwrap();

        println!("after: {:?}", decompressed_key);

        #[cfg(not(target_endian = "little"))]
        compile_error!("expected target to be little endian");
        // SAFETY: Arrays are always laid out in the obvious way. Any possible element value is
        // always valid. The pointee type has the same size, and the target of each transmute has
        // finer alignment than the source.
        // Although not a safety invariant, note that the guest target is always little-endian,
        // which was just sanity-checked, so this will always have the expected behavior.
        let decompressed_key_bytes =
            unsafe { core::mem::transmute::<&[u64; 12], &[u8; 12 * 8]>(&decompressed_key) };
        sp1_zkvm::io::commit_slice(decompressed_key_bytes);
    }
}
