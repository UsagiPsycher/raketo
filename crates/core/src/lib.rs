//! Core of raketo: printer catalogue, protocol frames, raster encoders
//! and parsing of printer notifications.

pub mod catalog;
pub mod encode;
pub mod frame;
pub mod notify;
pub mod raster;

pub use catalog::{lookup, models, Model};
pub use encode::{build_job, rasterize, Content, JobOptions, Raster};
