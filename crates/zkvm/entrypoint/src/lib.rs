#[cfg(target_os = "zkvm")]
use {
    cfg_if::cfg_if,
    syscalls::{syscall_hint_len, syscall_hint_read},
};

extern crate alloc;

#[cfg(target_os = "zkvm")]
pub mod allocators;

pub mod syscalls;

#[cfg(feature = "lib")]
pub mod io {
    pub use sp1_lib::io::*;
}

#[cfg(feature = "lib")]
pub mod lib {
    pub use sp1_lib::*;
}

#[cfg(all(target_os = "zkvm", feature = "libm"))]
mod libm;

/// The number of 32 bit words that the public values digest is composed of.
pub const PV_DIGEST_NUM_WORDS: usize = 8;
pub const POSEIDON_NUM_WORDS: usize = 8;

#[cfg(all(target_os = "zkvm", not(feature = "bump")))]
const MAX_MEMORY: usize = 1 << 37;

/// Size of the reserved region for input values with the embedded allocator.
#[cfg(all(target_os = "zkvm", not(feature = "bump")))]
pub(crate) const EMBEDDED_RESERVED_INPUT_REGION_SIZE: usize = 1 << 34;

/// Start of the reserved region for inputs with the embedded allocator.
#[cfg(all(target_os = "zkvm", not(feature = "bump")))]
pub(crate) const EMBEDDED_RESERVED_INPUT_START: usize =
    MAX_MEMORY - EMBEDDED_RESERVED_INPUT_REGION_SIZE;

/// Pointer to the current position in the reserved region for inputs with the embedded allocator.
#[cfg(all(target_os = "zkvm", not(feature = "bump")))]
static mut EMBEDDED_RESERVED_INPUT_PTR: usize = EMBEDDED_RESERVED_INPUT_START;

#[repr(C)]
pub struct ReadVecResult {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

/// Read a buffer from the input stream.
///
/// The buffer is read into uninitialized memory.
///
/// When the `bump` feature is enabled, the buffer is read into a new buffer allocated by the
/// program.
///
/// When there is no allocator selected, the program will fail to compile.
///
/// If the input stream is exhausted, the failed flag will be returned as true. In this case, the
/// other outputs from the function are likely incorrect, which is fine as `sp1-lib` always panics
/// in the case that the input stream is exhausted.
#[no_mangle]
pub extern "C" fn read_vec_raw() -> ReadVecResult {
    #[cfg(not(target_os = "zkvm"))]
    unreachable!("read_vec_raw should only be called on the zkvm target.");

    #[cfg(target_os = "zkvm")]
    {
        // Get the length of the input buffer.
        let len = syscall_hint_len();

        // If the length is u32::MAX, then the input stream is exhausted.
        if len == usize::MAX {
            return ReadVecResult { ptr: std::ptr::null_mut(), len: 0, capacity: 0 };
        }

        // Round up to multiple of 8 for whole-word alignment.
        let capacity = (len + 7) / 8 * 8;

        cfg_if! {
            if #[cfg(not(feature = "bump"))] {
                // Get the existing pointer in the reserved region which is the start of the vec.
                // Increment the pointer by the capacity to set the new pointer to the end of the vec.
                let ptr = unsafe { EMBEDDED_RESERVED_INPUT_PTR };
                if ptr.saturating_add(capacity) > MAX_MEMORY {
                    panic!("Input region overflowed.")
                }

                // SAFETY: The VM is single threaded.
                unsafe { EMBEDDED_RESERVED_INPUT_PTR += capacity };

                // Read the vec into uninitialized memory. The syscall assumes the memory is
                // uninitialized, which is true because the input ptr is incremented manually on each
                // read.
                syscall_hint_read(ptr as *mut u8, len);

                // Return the result.
                ReadVecResult { ptr: ptr as *mut u8, len, capacity }
            } else {
                // Allocate a buffer of the required length that is 8 byte aligned.
                let layout = std::alloc::Layout::from_size_align(capacity, 8).expect("vec is too large");

                // SAFETY: The layout was made through the checked constructor.
                let ptr = unsafe { std::alloc::alloc(layout) };

                // Read the vec into uninitialized memory. The syscall assumes the memory is
                // uninitialized, which is true because the bump allocator does not dealloc, so a new
                // alloc is always fresh.
                syscall_hint_read(ptr as *mut u8, len);

                // Return the result.
                ReadVecResult {
                    ptr: ptr as *mut u8,
                    len,
                    capacity,
                }

            }
        }
    }
}

#[cfg(target_os = "zkvm")]
mod zkvm {
    use crate::syscalls::syscall_halt;

    use cfg_if::cfg_if;
    use sha2::{Digest, Sha256};

    cfg_if! {
        if #[cfg(feature = "verify")] {
            use sp1_primitives::SP1Field;
            use slop_algebra::AbstractField;

            pub static mut DEFERRED_PROOFS_DIGEST: Option<[SP1Field; 8]> = None;
        }
    }

    cfg_if! {
        if #[cfg(feature = "blake3")] {
            pub static mut PUBLIC_VALUES_HASHER: Option<blake3::Hasher> = None;
        }
        else {
            pub static mut PUBLIC_VALUES_HASHER: Option<Sha256> = None;
        }
    }

    /// The ELF note values.
    const NAME: [u8; 8] = *b"SUCCINCT";
    const NAMESZ_LE: [u8; 4] = (NAME.len() as u32).to_le_bytes();
    const DESC: [u8; 4] = [b'1', 0, 0, 0];
    const DESCSZ_LE: [u8; 4] = (1u32).to_le_bytes();
    const TYPE_LE: [u8; 4] =
        (sp1_primitives::consts::NOTE_UNTRUSTED_PROGRAM_ENABLED as u32).to_le_bytes();

    #[cfg(feature = "untrusted_programs")]
    #[link_section = ".note.succinct"]
    #[used]
    pub static ELF_NOTE: [u8; 24] = [
        // header
        NAMESZ_LE[0],
        NAMESZ_LE[1],
        NAMESZ_LE[2],
        NAMESZ_LE[3],
        DESCSZ_LE[0],
        DESCSZ_LE[1],
        DESCSZ_LE[2],
        DESCSZ_LE[3],
        TYPE_LE[0],
        TYPE_LE[1],
        TYPE_LE[2],
        TYPE_LE[3],
        // name (8)
        NAME[0],
        NAME[1],
        NAME[2],
        NAME[3],
        NAME[4],
        NAME[5],
        NAME[6],
        NAME[7],
        // desc (4)
        DESC[0],
        DESC[1],
        DESC[2],
        DESC[3],
    ];

    #[no_mangle]
    unsafe extern "C" fn __start() {
        {
            #[cfg(all(target_os = "zkvm", not(feature = "bump")))]
            crate::allocators::init();

            cfg_if::cfg_if! {
                if #[cfg(feature = "blake3")] {
                    PUBLIC_VALUES_HASHER = Some(blake3::Hasher::new());
                }
                else {
                    PUBLIC_VALUES_HASHER = Some(Sha256::new());
                }
            }

            #[cfg(feature = "verify")]
            {
                DEFERRED_PROOFS_DIGEST = Some([SP1Field::zero(); 8]);
            }

            extern "C" {
                fn main();
            }
            main()
        }

        syscall_halt(0);
    }

    // core::arch::global_asm!(include_str!("memset.s"));
    core::arch::global_asm!(include_str!("memcpy.s"));

    // Alias the stack top to a static we can load easily.
    static _STACK_TOP: u64 = sp1_primitives::consts::STACK_TOP;
    core::arch::global_asm!(
        r#"
    .section .text._start;
    .globl _start;
    _start:
        .option push;
        .option norelax;
        la gp, __global_pointer$;
        .option pop;
        la sp, {0}
        ld sp, 0(sp)
        call __start;
    "#,
        sym _STACK_TOP
    );

    pub fn zkvm_getrandom(s: &mut [u8]) -> Result<(), getrandom_v2::Error> {
        unsafe {
            crate::syscalls::sys_rand(s.as_mut_ptr(), s.len());
        }

        Ok(())
    }

    getrandom_v2::register_custom_getrandom!(zkvm_getrandom);

    #[no_mangle]
    unsafe extern "Rust" fn __getrandom_v03_custom(
        dest: *mut u8,
        len: usize,
    ) -> Result<(), getrandom_v3::Error> {
        unsafe {
            crate::syscalls::sys_rand(dest, len);
        }

        Ok(())
    }
}

#[macro_export]
macro_rules! entrypoint {
    ($path:path) => {
        const ZKVM_ENTRY: fn() = $path;

        mod zkvm_generated_main {

            #[no_mangle]
            fn main() {
                // Link to the actual entrypoint only when compiling for zkVM, otherwise run a
                // simple noop. Doing this avoids compilation errors when building for the host
                // target.
                //
                // Note that, however, it's generally considered wasted effort compiling zkVM
                // programs against the host target. This just makes it such that doing so wouldn't
                // result in an error, which can happen when building a Cargo workspace containing
                // zkVM program crates.
                if cfg!(target_os = "zkvm") {
                    super::ZKVM_ENTRY()
                } else {
                    eprintln!("Not running in zkVM, skipping entrypoint");
                }
            }
        }
    };
}
