//! Real deterministic ZIP package helper for storage/control-plane tests.
use std::io::{Cursor, Write};

/// Build a ZIP containing a single ordinary file with the supplied contents.
pub fn minimal_zip(contents: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "fixture.txt",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(contents).unwrap();
    writer.finish().unwrap().into_inner()
}
