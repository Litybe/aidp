//! Closed sets of this crate, one enum per file.

#[macro_use]
mod string_enum;

pub mod distill_trigger;
pub mod domain;
pub mod memory_kind;
pub mod memory_status;
pub mod note_status;
pub mod ref_type;
pub mod route;
pub mod source;

pub use distill_trigger::DistillTrigger;
pub use domain::Domain;
pub use memory_kind::MemoryKind;
pub use memory_status::MemoryStatus;
pub use note_status::NoteStatus;
pub use ref_type::RefType;
pub use route::Route;
pub use source::Source;
