mod decode;
mod ir;
mod memory;
mod vm;

pub use memory::{MEMORY_SIZE, PAGE_SIZE};
pub use vm::{Error, RunResult, Vm};
