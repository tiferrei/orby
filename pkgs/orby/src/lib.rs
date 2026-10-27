pub mod config;
pub mod machine;
pub mod proxy;

#[cfg(any(target_os = "macos", test))]
pub mod activation;
#[cfg(target_os = "macos")]
pub mod launchd;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub fn error(message: impl Into<String>) -> Error {
    std::io::Error::other(message.into()).into()
}
