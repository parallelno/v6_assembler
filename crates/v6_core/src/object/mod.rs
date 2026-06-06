//! Relocatable object output (ELF32) support.

pub mod elf;
pub mod section;

pub use section::{Reloc, RelocKind, RelocTarget, Section};
