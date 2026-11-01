//! A real Linux process that deliberately never speaks the Lambda Runtime API.
//!
//! The Docker negative fixture must reach the production initialization deadline
//! while this process is alive. It has no runtime client or network operations.

#![forbid(unsafe_code)]

use std::{thread::sleep, time::Duration};

fn main() {
    sleep(Duration::from_secs(300));
}
