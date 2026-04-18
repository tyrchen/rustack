//! Hand-written XML serialization and deserialization.
//!
//! CloudFront restXml is simple enough that we write targeted emitters per
//! response shape (as opposed to a generic serde-based layer). This keeps
//! wire fidelity tight and compile times low.

pub mod de;
pub mod ser;
pub mod writer;

pub use writer::XmlWriter;
