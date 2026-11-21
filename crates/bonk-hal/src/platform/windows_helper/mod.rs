use std::io;

use crate::image::Access;

mod client;
mod server;

pub(in crate::platform) use client::*;
pub(in crate::platform) use server::*;

const HELPER_COMMAND: &str = "elevated-helper";

fn access_name(access: Access) -> &'static str {
    match access {
        Access::ReadOnly => "ro",
        Access::ReadWrite => "rw",
    }
}

fn denied(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}
