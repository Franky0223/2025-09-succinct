use deepsize2::DeepSizeOf;
use enum_map::Enum;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

use crate::{events::FieldOperation, RiscvAirId};

/// System Calls.
///
/// A system call is invoked by the the `ecall` instruction with a specific value in register t0.
/// The syscall number is a 32-bit integer with the following little-endian layout:
///
/// | Byte 0 | Byte 1 | Byte 2 | Byte 3 |
/// | ------ | ------ | ------ | ------ |
/// |   ID   | Table  | Unused | Unused |
///
/// where:
/// - Byte 0: The system call identifier.
/// - Byte 1: Whether the handler of the system call has its own table. This is used in the CPU
///   table to determine whether to lookup the syscall using the syscall interaction.
/// - Byte 2: Previously, this was the number of additional cycles the syscall uses. Now, this byte
///   is unused, as each syscall instruction increments the clock by 256 additionally.
/// - Byte 3: Currently unused.
#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    EnumIter,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    Enum,
    Default,
    DeepSizeOf,
)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
#[repr(u32)]
pub enum SyscallCode {
    /// Halts the program.
    #[default]
    HALT = 0x00_00_00_00,

    /// Write to the output buffer.
    WRITE = 0x00_00_00_02,

    /// Enter unconstrained block.
    ENTER_UNCONSTRAINED = 0x00_00_00_03,

    /// Exit unconstrained block.
    EXIT_UNCONSTRAINED = 0x00_00_00_04,

    /// Executes the `SHA_EXTEND` precompile.
    SHA_EXTEND = 0x00_30_01_05,

    /// Executes the `SHA_COMPRESS` precompile.
    SHA_COMPRESS = 0x00_01_01_06,

    /// Executes the `ED_ADD` precompile.
    ED_ADD = 0x00_01_01_07,

    /// Executes the `ED_DECOMPRESS` precompile.
    ED_DECOMPRESS = 0x00_00_01_08,

    /// Executes the `KECCAK_PERMUTE` precompile.
    KECCAK_PERMUTE = 0x00_01_01_09,

    /// Executes the `SECP256K1_ADD` precompile.
    SECP256K1_ADD = 0x00_01_01_0A,

    /// Executes the `SECP256K1_DOUBLE` precompile.
    SECP256K1_DOUBLE = 0x00_00_01_0B,

    /// Executes the `SECP256K1_DECOMPRESS` precompile.
    SECP256K1_DECOMPRESS = 0x00_00_01_0C,

    /// Executes the `BN254_ADD` precompile.
    BN254_ADD = 0x00_01_01_0E,

    /// Executes the `BN254_DOUBLE` precompile.
    BN254_DOUBLE = 0x00_00_01_0F,

    /// Executes the `COMMIT` precompile.
    COMMIT = 0x00_00_00_10,

    /// Executes the `COMMIT_DEFERRED_PROOFS` precompile.
    COMMIT_DEFERRED_PROOFS = 0x00_00_00_1A,

    /// Executes the `VERIFY_SP1_PROOF` precompile.
    VERIFY_SP1_PROOF = 0x00_00_00_1B,

    /// Executes the `BLS12381_DECOMPRESS` precompile.
    BLS12381_DECOMPRESS = 0x00_00_01_1C,

    /// Executes the `HINT_LEN` precompile.
    HINT_LEN = 0x00_00_00_F0,

    /// Executes the `HINT_READ` precompile.
    HINT_READ = 0x00_00_00_F1,

    /// Executes the `UINT256_MUL` precompile.
    UINT256_MUL = 0x00_01_01_1D,

    /// Executes the `U256XU2048_MUL` precompile.
    U256XU2048_MUL = 0x00_01_01_2F,

    /// Executes the `BLS12381_ADD` precompile.
    BLS12381_ADD = 0x00_01_01_1E,

    /// Executes the `BLS12381_DOUBLE` precompile.
    BLS12381_DOUBLE = 0x00_00_01_1F,

    /// Executes the `BLS12381_FP_ADD` precompile.
    BLS12381_FP_ADD = 0x00_01_01_20,

    /// Executes the `BLS12381_FP_SUB` precompile.
    BLS12381_FP_SUB = 0x00_01_01_21,

    /// Executes the `BLS12381_FP_MUL` precompile.
    BLS12381_FP_MUL = 0x00_01_01_22,

    /// Executes the `BLS12381_FP2_ADD` precompile.
    BLS12381_FP2_ADD = 0x00_01_01_23,

    /// Executes the `BLS12381_FP2_SUB` precompile.
    BLS12381_FP2_SUB = 0x00_01_01_24,

    /// Executes the `BLS12381_FP2_MUL` precompile.
    BLS12381_FP2_MUL = 0x00_01_01_25,

    /// Executes the `BN254_FP_ADD` precompile.
    BN254_FP_ADD = 0x00_01_01_26,

    /// Executes the `BN254_FP_SUB` precompile.
    BN254_FP_SUB = 0x00_01_01_27,

    /// Executes the `BN254_FP_MUL` precompile.
    BN254_FP_MUL = 0x00_01_01_28,

    /// Executes the `BN254_FP2_ADD` precompile.
    BN254_FP2_ADD = 0x00_01_01_29,

    /// Executes the `BN254_FP2_SUB` precompile.
    BN254_FP2_SUB = 0x00_01_01_2A,

    /// Executes the `BN254_FP2_MUL` precompile.
    BN254_FP2_MUL = 0x00_01_01_2B,

    /// Executes the `SECP256R1_ADD` precompile.
    SECP256R1_ADD = 0x00_01_01_2C,

    /// Executes the `SECP256R1_DOUBLE` precompile.
    SECP256R1_DOUBLE = 0x00_00_01_2D,

    /// Executes the `SECP256R1_DECOMPRESS` precompile.
    SECP256R1_DECOMPRESS = 0x00_00_01_2E,

    /// Executes the `UINT256_ADD_CARRY` precompile.
    UINT256_ADD_CARRY = 0x00_01_01_30,

    /// Executes the `UINT256_MUL_CARRY` precompile.
    UINT256_MUL_CARRY = 0x00_01_01_31,

    /// Executes the `MPROTECT` syscall.
    #[allow(clippy::mistyped_literal_suffixes)]
    MPROTECT = 0x00_00_01_32,

    /// Executes the `POSEIDON2` syscall.
    POSEIDON2 = 0x00_00_01_33,
}

impl SyscallCode {
    /// Create a [`SyscallCode`] from a u32.
    #[must_use]
    pub fn from_u32(value: u32) -> Self {
        match value {
            0x00_00_00_00 => SyscallCode::HALT,
            0x00_00_00_02 => SyscallCode::WRITE,
            0x00_00_00_03 => SyscallCode::ENTER_UNCONSTRAINED,
            0x00_00_00_04 => SyscallCode::EXIT_UNCONSTRAINED,
            0x00_30_01_05 => SyscallCode::SHA_EXTEND,
            0x00_01_01_06 => SyscallCode::SHA_COMPRESS,
            0x00_01_01_07 => SyscallCode::ED_ADD,
            0x00_00_01_08 => SyscallCode::ED_DECOMPRESS,
            0x00_01_01_09 => SyscallCode::KECCAK_PERMUTE,
            0x00_01_01_0A => SyscallCode::SECP256K1_ADD,
            0x00_00_01_0B => SyscallCode::SECP256K1_DOUBLE,
            0x00_00_01_0C => SyscallCode::SECP256K1_DECOMPRESS,
            0x00_01_01_0E => SyscallCode::BN254_ADD,
            0x00_00_01_0F => SyscallCode::BN254_DOUBLE,
            0x00_01_01_1E => SyscallCode::BLS12381_ADD,
            0x00_00_01_1F => SyscallCode::BLS12381_DOUBLE,
            0x00_00_00_10 => SyscallCode::COMMIT,
            0x00_00_00_1A => SyscallCode::COMMIT_DEFERRED_PROOFS,
            0x00_00_00_1B => SyscallCode::VERIFY_SP1_PROOF,
            0x00_00_00_F0 => SyscallCode::HINT_LEN,
            0x00_00_00_F1 => SyscallCode::HINT_READ,
            0x00_01_01_1D => SyscallCode::UINT256_MUL,
            0x00_01_01_2F => SyscallCode::U256XU2048_MUL,
            0x00_01_01_20 => SyscallCode::BLS12381_FP_ADD,
            0x00_01_01_21 => SyscallCode::BLS12381_FP_SUB,
            0x00_01_01_22 => SyscallCode::BLS12381_FP_MUL,
            0x00_01_01_23 => SyscallCode::BLS12381_FP2_ADD,
            0x00_01_01_24 => SyscallCode::BLS12381_FP2_SUB,
            0x00_01_01_25 => SyscallCode::BLS12381_FP2_MUL,
            0x00_01_01_26 => SyscallCode::BN254_FP_ADD,
            0x00_01_01_27 => SyscallCode::BN254_FP_SUB,
            0x00_01_01_28 => SyscallCode::BN254_FP_MUL,
            0x00_01_01_29 => SyscallCode::BN254_FP2_ADD,
            0x00_01_01_2A => SyscallCode::BN254_FP2_SUB,
            0x00_01_01_2B => SyscallCode::BN254_FP2_MUL,
            0x00_00_01_1C => SyscallCode::BLS12381_DECOMPRESS,
            0x00_01_01_2C => SyscallCode::SECP256R1_ADD,
            0x00_00_01_2D => SyscallCode::SECP256R1_DOUBLE,
            0x00_00_01_2E => SyscallCode::SECP256R1_DECOMPRESS,
            0x00_01_01_30 => SyscallCode::UINT256_ADD_CARRY,
            0x00_01_01_31 => SyscallCode::UINT256_MUL_CARRY,
            #[allow(clippy::mistyped_literal_suffixes)]
            0x00_00_01_32 => SyscallCode::MPROTECT,
            0x00_00_01_33 => SyscallCode::POSEIDON2,
            _ => panic!("invalid syscall number: {value}"),
        }
    }

    /// Get the system call identifier.
    #[must_use]
    pub fn syscall_id(self) -> u32 {
        (self as u32).to_le_bytes()[0].into()
    }

    /// Get whether the handler of the system call has its own table.
    #[must_use]
    pub fn should_send(self) -> u32 {
        (self as u32).to_le_bytes()[1].into()
    }

    /// Map a syscall to another one in order to coalesce their counts.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn count_map(&self) -> Self {
        match self {
            SyscallCode::BN254_FP_SUB => SyscallCode::BN254_FP_ADD,
            SyscallCode::BN254_FP_MUL => SyscallCode::BN254_FP_ADD,
            SyscallCode::BN254_FP2_SUB => SyscallCode::BN254_FP2_ADD,
            SyscallCode::BLS12381_FP_SUB => SyscallCode::BLS12381_FP_ADD,
            SyscallCode::BLS12381_FP_MUL => SyscallCode::BLS12381_FP_ADD,
            SyscallCode::BLS12381_FP2_SUB => SyscallCode::BLS12381_FP2_ADD,
            _ => *self,
        }
    }

    /// Map a syscall to a field operation.
    #[must_use]
    pub fn fp_op_map(&self) -> FieldOperation {
        match self {
            SyscallCode::BLS12381_FP_ADD
            | SyscallCode::BLS12381_FP2_ADD
            | SyscallCode::BN254_FP_ADD
            | SyscallCode::BN254_FP2_ADD => FieldOperation::Add,
            SyscallCode::BLS12381_FP_SUB
            | SyscallCode::BLS12381_FP2_SUB
            | SyscallCode::BN254_FP_SUB
            | SyscallCode::BN254_FP2_SUB => FieldOperation::Sub,
            SyscallCode::BLS12381_FP_MUL
            | SyscallCode::BLS12381_FP2_MUL
            | SyscallCode::BN254_FP_MUL
            | SyscallCode::BN254_FP2_MUL => FieldOperation::Mul,
            _ => unreachable!(),
        }
    }

    /// Map a syscall to a uint256 operation.
    #[must_use]
    pub fn uint256_op_map(&self) -> crate::events::Uint256Operation {
        match self {
            SyscallCode::UINT256_ADD_CARRY => crate::events::Uint256Operation::Add,
            SyscallCode::UINT256_MUL_CARRY => crate::events::Uint256Operation::Mul,
            _ => unreachable!(),
        }
    }

    /// Get the ID of the AIR used in the syscall implementation.
    #[must_use]
    pub fn as_air_id(self) -> Option<RiscvAirId> {
        Some(match self {
            SyscallCode::SHA_EXTEND => RiscvAirId::ShaExtend,
            SyscallCode::SHA_COMPRESS => RiscvAirId::ShaCompress,
            SyscallCode::ED_ADD => RiscvAirId::EdAddAssign,
            SyscallCode::ED_DECOMPRESS => RiscvAirId::EdDecompress,
            SyscallCode::KECCAK_PERMUTE => RiscvAirId::KeccakPermute,
            SyscallCode::SECP256K1_ADD => RiscvAirId::Secp256k1AddAssign,
            SyscallCode::SECP256K1_DOUBLE => RiscvAirId::Secp256k1DoubleAssign,
            SyscallCode::SECP256K1_DECOMPRESS => RiscvAirId::Secp256k1Decompress,
            SyscallCode::BN254_ADD => RiscvAirId::Bn254AddAssign,
            SyscallCode::BN254_DOUBLE => RiscvAirId::Bn254DoubleAssign,
            SyscallCode::BLS12381_DECOMPRESS => RiscvAirId::Bls12381Decompress,
            SyscallCode::UINT256_MUL => RiscvAirId::Uint256MulMod,
            SyscallCode::U256XU2048_MUL => RiscvAirId::U256XU2048Mul,
            SyscallCode::BLS12381_ADD => RiscvAirId::Bls12381AddAssign,
            SyscallCode::BLS12381_DOUBLE => RiscvAirId::Bls12381DoubleAssign,
            SyscallCode::BLS12381_FP_ADD
            | SyscallCode::BLS12381_FP_SUB
            | SyscallCode::BLS12381_FP_MUL => RiscvAirId::Bls12381FpOpAssign,
            SyscallCode::BLS12381_FP2_ADD | SyscallCode::BLS12381_FP2_SUB => {
                RiscvAirId::Bls12381Fp2AddSubAssign
            }
            SyscallCode::BLS12381_FP2_MUL => RiscvAirId::Bls12381Fp2MulAssign,
            SyscallCode::BN254_FP_ADD | SyscallCode::BN254_FP_SUB | SyscallCode::BN254_FP_MUL => {
                RiscvAirId::Bn254FpOpAssign
            }
            SyscallCode::BN254_FP2_ADD | SyscallCode::BN254_FP2_SUB => {
                RiscvAirId::Bn254Fp2AddSubAssign
            }
            SyscallCode::BN254_FP2_MUL => RiscvAirId::Bn254Fp2MulAssign,
            SyscallCode::SECP256R1_ADD => RiscvAirId::Secp256r1AddAssign,
            SyscallCode::SECP256R1_DOUBLE => RiscvAirId::Secp256r1DoubleAssign,
            SyscallCode::SECP256R1_DECOMPRESS => RiscvAirId::Secp256r1Decompress,
            SyscallCode::UINT256_ADD_CARRY | SyscallCode::UINT256_MUL_CARRY => {
                RiscvAirId::Uint256Ops
            }
            SyscallCode::MPROTECT => RiscvAirId::Mprotect,
            SyscallCode::POSEIDON2 => RiscvAirId::Poseidon2,
            SyscallCode::HALT
            | SyscallCode::WRITE
            | SyscallCode::ENTER_UNCONSTRAINED
            | SyscallCode::EXIT_UNCONSTRAINED
            | SyscallCode::COMMIT
            | SyscallCode::COMMIT_DEFERRED_PROOFS
            | SyscallCode::VERIFY_SP1_PROOF
            | SyscallCode::HINT_LEN
            | SyscallCode::HINT_READ => return None,
        })
    }

    /// The maximum number of touched words for each syscall code.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn touched_addresses(&self) -> usize {
        // To get this number, either analyze the number of touched memory in the executor of the
        // syscall, or run a program with the syscall and analyze the ratio of heights of the
        // `MemoryLocal` table and the corresponding syscall table in the precompile shard.
        match self {
            SyscallCode::SHA_EXTEND => 64,
            SyscallCode::SHA_COMPRESS => 72,
            SyscallCode::KECCAK_PERMUTE => 25,
            SyscallCode::BLS12381_ADD
            | SyscallCode::BLS12381_FP2_ADD
            | SyscallCode::BLS12381_FP2_SUB
            | SyscallCode::BLS12381_FP2_MUL => 24,
            SyscallCode::BLS12381_DECOMPRESS
            | SyscallCode::BLS12381_DOUBLE
            | SyscallCode::BLS12381_FP_ADD
            | SyscallCode::BLS12381_FP_SUB
            | SyscallCode::BLS12381_FP_MUL => 12,
            SyscallCode::ED_ADD
            | SyscallCode::SECP256K1_ADD
            | SyscallCode::SECP256R1_ADD
            | SyscallCode::BN254_ADD
            | SyscallCode::BN254_FP2_ADD
            | SyscallCode::BN254_FP2_SUB
            | SyscallCode::BN254_FP2_MUL => 16,
            SyscallCode::ED_DECOMPRESS
            | SyscallCode::SECP256K1_DECOMPRESS
            | SyscallCode::SECP256R1_DECOMPRESS
            | SyscallCode::SECP256K1_DOUBLE
            | SyscallCode::SECP256R1_DOUBLE
            | SyscallCode::BN254_DOUBLE
            | SyscallCode::BN254_FP_ADD
            | SyscallCode::BN254_FP_SUB
            | SyscallCode::BN254_FP_MUL => 8,
            SyscallCode::UINT256_MUL => 12,
            SyscallCode::UINT256_ADD_CARRY | SyscallCode::UINT256_MUL_CARRY => 20,
            SyscallCode::U256XU2048_MUL => 72,
            SyscallCode::MPROTECT => 0,
            SyscallCode::POSEIDON2 => 8,
            _ => 0,
        }
    }

    /// The maximum number of touched pages for each syscall code.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn touched_pages(&self) -> usize {
        // To get this number, either analyze the number of touched pages in the executor of the
        // syscall, or check the number of `AddressSlicePageProtOperation` in the corresponding AIR.
        // This function assumes that each slice touches two pages, which is the maximum.
        match self {
            SyscallCode::SHA_EXTEND => 2 * 2,
            SyscallCode::SHA_COMPRESS => 3 * 2,
            SyscallCode::KECCAK_PERMUTE => 2 * 2,
            SyscallCode::BLS12381_ADD
            | SyscallCode::SECP256K1_ADD
            | SyscallCode::SECP256R1_ADD
            | SyscallCode::BN254_ADD
            | SyscallCode::ED_ADD
            | SyscallCode::BLS12381_DECOMPRESS
            | SyscallCode::SECP256K1_DECOMPRESS
            | SyscallCode::SECP256R1_DECOMPRESS
            | SyscallCode::ED_DECOMPRESS => 2 * 2,

            SyscallCode::BLS12381_DOUBLE
            | SyscallCode::SECP256K1_DOUBLE
            | SyscallCode::SECP256R1_DOUBLE
            | SyscallCode::BN254_DOUBLE => 2,

            SyscallCode::BLS12381_FP2_ADD
            | SyscallCode::BLS12381_FP2_SUB
            | SyscallCode::BLS12381_FP2_MUL
            | SyscallCode::BLS12381_FP_ADD
            | SyscallCode::BLS12381_FP_SUB
            | SyscallCode::BLS12381_FP_MUL
            | SyscallCode::BN254_FP2_ADD
            | SyscallCode::BN254_FP2_SUB
            | SyscallCode::BN254_FP2_MUL
            | SyscallCode::BN254_FP_ADD
            | SyscallCode::BN254_FP_SUB
            | SyscallCode::BN254_FP_MUL => 2 * 2,

            SyscallCode::UINT256_MUL => 2 * 2,
            SyscallCode::UINT256_ADD_CARRY | SyscallCode::UINT256_MUL_CARRY => 5 * 2,
            SyscallCode::U256XU2048_MUL => 4 * 2,
            SyscallCode::MPROTECT => 1,
            SyscallCode::POSEIDON2 => 2,
            _ => 0,
        }
    }
}

impl std::fmt::Display for SyscallCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
