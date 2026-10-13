pub mod azure_devops;
pub mod github;
mod limits;
pub(crate) mod message;
pub(crate) mod time;

#[cfg(test)]
pub(crate) mod fake;
