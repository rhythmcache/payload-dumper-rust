#[cfg(feature = "local_zip")]
pub mod local_zip;

#[cfg(feature = "remote_ota")]
pub mod remote_zip;

#[cfg(all(feature = "remote_ota", feature = "local_zip"))]
pub mod zip_core;
