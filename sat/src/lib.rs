//! SAT: native host for archive-registered WebAssembly components.

pub mod access;

pub mod cli;

pub mod config;

mod files;

mod grants;

mod http;

pub mod metadata;

pub mod registry;

pub mod runtime;
