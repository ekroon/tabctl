//! Field-selective GraphQL queries and mutations over the tabctl host.
//! Execution preserves partial data and resolver errors; callers must treat any errors as failure.

mod context;
mod convert;
mod execution;
mod response;
mod schema;
mod types;

pub use context::CommandSender;
pub use execution::execute;

/// Export the GraphQL schema for client discovery.
pub fn schema_sdl() -> String {
    schema::create_schema().as_sdl()
}
