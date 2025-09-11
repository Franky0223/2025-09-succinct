use deepsize2::DeepSizeOf;
use serde::{Deserialize, Serialize};

use sp1_curves::{
    params::{NumLimbs, NumWords},
    weierstrass::{
        bls12_381::bls12381_decompress, secp256k1::secp256k1_decompress,
        secp256r1::secp256r1_decompress,
    },
    AffinePoint, CurveType, EllipticCurve,
};
use sp1_primitives::consts::{bytes_to_words_le_vec, words_to_bytes_le_vec};
use typenum::Unsigned;

use crate::{
    events::{
        memory::{MemoryReadRecord, MemoryWriteRecord},
        MemoryLocalEvent, PageProtLocalEvent, PageProtRecord,
    },
    syscalls::SyscallContext,
    ExecutorConfig,
};

/// Elliptic Curve Page Prot Records.
#[derive(Default, Debug, Clone, Serialize, Deserialize, DeepSizeOf)]
pub struct EllipticCurvePageProtRecords {
    /// The page prot records for reading the address.
    pub read_page_prot_records: Vec<PageProtRecord>,
    /// The page prot records for writing the address.
    pub write_page_prot_records: Vec<PageProtRecord>,
}

/// Elliptic Curve Add Event.
///
/// This event is emitted when an elliptic curve addition operation is performed.
#[derive(Default, Debug, Clone, Serialize, Deserialize, DeepSizeOf)]
pub struct EllipticCurveAddEvent {
    /// The clock cycle.
    pub clk: u64,
    /// The pointer to the first point.
    pub p_ptr: u64,
    /// The first point as a list of words.
    pub p: Vec<u64>,
    /// The pointer to the second point.
    pub q_ptr: u64,
    /// The second point as a list of words.
    pub q: Vec<u64>,
    /// The memory records for the first point.
    pub p_memory_records: Vec<MemoryWriteRecord>,
    /// The memory records for the second point.
    pub q_memory_records: Vec<MemoryReadRecord>,
    /// The local memory access records.
    pub local_mem_access: Vec<MemoryLocalEvent>,
    /// The page prot records.
    pub page_prot_records: EllipticCurvePageProtRecords,
    /// The local page prot access records.
    pub local_page_prot_access: Vec<PageProtLocalEvent>,
}

/// Elliptic Curve Double Event.
///
/// This event is emitted when an elliptic curve doubling operation is performed.
#[derive(Default, Debug, Clone, Serialize, Deserialize, DeepSizeOf)]
pub struct EllipticCurveDoubleEvent {
    /// The clock cycle.
    pub clk: u64,
    /// The pointer to the point.
    pub p_ptr: u64,
    /// The point as a list of words.
    pub p: Vec<u64>,
    /// The memory records for the point.
    pub p_memory_records: Vec<MemoryWriteRecord>,
    /// The local memory access records.
    pub local_mem_access: Vec<MemoryLocalEvent>,
    /// Write slice page prot access records.
    pub write_slice_page_prot_access: Vec<PageProtRecord>,
    /// The local page prot access records.
    pub local_page_prot_access: Vec<PageProtLocalEvent>,
}

/// Elliptic Curve Point Decompress Event.
///
/// This event is emitted when an elliptic curve point decompression operation is performed.
#[derive(Default, Debug, Clone, Serialize, Deserialize, DeepSizeOf)]
pub struct EllipticCurveDecompressEvent {
    /// The clock cycle.
    pub clk: u64,
    /// The pointer to the point.
    pub ptr: u64,
    /// The sign bit of the point.
    pub sign_bit: bool,
    /// The x coordinate as a list of bytes.
    pub x_bytes: Vec<u8>,
    /// The decompressed y coordinate as a list of bytes.
    pub decompressed_y_bytes: Vec<u8>,
    /// The memory records for the x coordinate.
    pub x_memory_records: Vec<MemoryReadRecord>,
    /// The memory records for the y coordinate.
    pub y_memory_records: Vec<MemoryWriteRecord>,
    /// The local memory access records.
    pub local_mem_access: Vec<MemoryLocalEvent>,
    /// The page prot records.
    pub page_prot_records: EllipticCurvePageProtRecords,
    /// The local page prot access records.
    pub local_page_prot_access: Vec<PageProtLocalEvent>,
}

/// Create an elliptic curve add event. It takes two pointers to memory locations, reads the points
/// from memory, adds them together, and writes the result back to the first memory location.
/// The generic parameter `N` is the number of u32 words in the point representation. For example,
/// for the secp256k1 curve, `N` would be 16 (64 bytes) because the x and y coordinates are 32 bytes
/// each.
pub fn create_ec_add_event<E: EllipticCurve, Ex: ExecutorConfig>(
    rt: &mut SyscallContext<'_, '_, Ex>,
    arg1: u64,
    arg2: u64,
) -> EllipticCurveAddEvent {
    let start_clk = rt.clk;
    let p_ptr = arg1;
    assert!(p_ptr.is_multiple_of(8), "p_ptr must be 8-byte aligned");
    let q_ptr = arg2;
    assert!(q_ptr.is_multiple_of(8), "q_ptr must be 8-byte aligned");

    let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

    let p = rt.slice_unsafe(p_ptr, num_words);

    let (q_memory_records, q, read_page_prot_records) = rt.mr_slice(q_ptr, num_words);

    // When we write to p, we want the clk to be incremented because p and q could be the same.
    rt.clk += 1;

    let p_affine = AffinePoint::<E>::from_words_le(&p);
    let q_affine = AffinePoint::<E>::from_words_le(&q);
    let result_affine = p_affine + q_affine;

    let result_words = result_affine.to_words_le();

    let (p_memory_records, write_page_prot_records) = rt.mw_slice(p_ptr, &result_words, true);

    let (local_mem_access, local_page_prot_access) = rt.postprocess();

    EllipticCurveAddEvent {
        clk: start_clk,
        p_ptr,
        p,
        q_ptr,
        q,
        p_memory_records,
        q_memory_records,
        local_mem_access,
        page_prot_records: EllipticCurvePageProtRecords {
            read_page_prot_records,
            write_page_prot_records,
        },
        local_page_prot_access,
    }
}

/// Create an elliptic curve double event.
///
/// It takes a pointer to a memory location, reads the point from memory, doubles it, and writes the
/// result back to the memory location.
pub fn create_ec_double_event<E: EllipticCurve, Ex: ExecutorConfig>(
    rt: &mut SyscallContext<'_, '_, Ex>,
    arg1: u64,
    _: u64,
) -> EllipticCurveDoubleEvent {
    let start_clk = rt.clk;
    let p_ptr = arg1;
    assert!(p_ptr.is_multiple_of(8), "p_ptr must be 8-byte aligned");

    let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

    let p = rt.slice_unsafe(p_ptr, num_words);

    let p_affine = AffinePoint::<E>::from_words_le(&p);

    let result_affine = E::ec_double(&p_affine);

    let result_words = result_affine.to_words_le();

    let (p_memory_records, write_page_prot_records) = rt.mw_slice(p_ptr, &result_words, true);

    let (local_mem_access, local_page_prot_access) = rt.postprocess();

    EllipticCurveDoubleEvent {
        clk: start_clk,
        p_ptr,
        p,
        p_memory_records,
        local_mem_access,
        write_slice_page_prot_access: write_page_prot_records,
        local_page_prot_access,
    }
}

/// Create an elliptic curve decompress event.
///
/// It takes a pointer to a memory location, reads the point from memory, decompresses it, and
/// writes the result back to the memory location.
pub fn create_ec_decompress_event<E: EllipticCurve, Ex: ExecutorConfig>(
    rt: &mut SyscallContext<'_, '_, Ex>,
    slice_ptr: u64,
    sign_bit: u64,
) -> EllipticCurveDecompressEvent {
    let start_clk = rt.clk;
    assert!(slice_ptr.is_multiple_of(8), "slice_ptr must be 8-byte aligned");
    assert!(sign_bit <= 1, "is_odd must be 0 or 1");

    let num_limbs = <E::BaseField as NumLimbs>::Limbs::USIZE;
    let num_words_field_element = num_limbs / 8;

    let (x_memory_records, x_vec, read_page_prot_records) =
        rt.mr_slice(slice_ptr + (num_limbs as u64), num_words_field_element);

    let x_bytes = words_to_bytes_le_vec(&x_vec);
    let mut x_bytes_be = x_bytes.clone();
    x_bytes_be.reverse();

    let decompress_fn = match E::CURVE_TYPE {
        CurveType::Secp256k1 => secp256k1_decompress::<E>,
        CurveType::Secp256r1 => secp256r1_decompress::<E>,
        CurveType::Bls12381 => bls12381_decompress::<E>,
        _ => panic!("Unsupported curve"),
    };

    let computed_point: AffinePoint<E> = decompress_fn(&x_bytes_be, sign_bit as u32);

    let mut decompressed_y_bytes = computed_point.y.to_bytes_le();
    decompressed_y_bytes.resize(num_limbs, 0u8);
    let y_words = bytes_to_words_le_vec(&decompressed_y_bytes);

    // Increment clk because read and write could be on same page prot page
    rt.clk += 1;
    let (y_memory_records, write_page_prot_records) = rt.mw_slice(slice_ptr, &y_words, false);

    let (local_mem_access, local_page_prot_access) = rt.postprocess();

    EllipticCurveDecompressEvent {
        clk: start_clk,
        ptr: slice_ptr,
        sign_bit: sign_bit != 0,
        x_bytes,
        decompressed_y_bytes,
        x_memory_records,
        y_memory_records,
        local_mem_access,
        page_prot_records: EllipticCurvePageProtRecords {
            read_page_prot_records,
            write_page_prot_records,
        },
        local_page_prot_access,
    }
}
