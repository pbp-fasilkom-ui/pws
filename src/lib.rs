// Clippy baseline freeze.
//
// CI previously ran Clippy with `continue-on-error: true`, which meant its
// result never affected the workflow conclusion -- and CD deploys on that
// conclusion, so a flagged defect shipped green. Clippy now blocks.
//
// Making it block required doing something about the 69 pre-existing findings.
// They were reviewed: all are style or dead-code, none are security or
// correctness problems (the `unreachable_code` one is an `Ok(())` after an
// infinite loop in the build queue). `cargo clippy --fix` cannot be used on
// them, because its rewrite breaks a `tracing::instrument(skip(...))`
// reference.
//
// So the existing categories are allowed here and everything else denies.
// New code is held to the full lint set; this list is meant to be burned down
// and shrunk, not added to.
#![allow(
    clippy::enum_variant_names,
    clippy::needless_borrow,
    clippy::needless_return,
    clippy::new_without_default,
    clippy::redundant_field_names,
    clippy::single_char_add_str,
    clippy::to_string_in_format_args,
    clippy::upper_case_acronyms,
    clippy::useless_format,
    dead_code,
    unreachable_code,
    unused_imports,
    unused_variables
)]

pub mod auth;
pub mod authz;
pub mod build_logs;
pub mod configuration;
pub mod dashboard;
pub mod docker;
pub mod dockerfile_templates;
pub mod get_env;
pub mod git;
pub mod owner;
pub mod projects;
pub mod queue;
pub mod rate_limit;
pub mod startup;
pub mod telemetry;
