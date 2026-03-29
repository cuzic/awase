pub mod config;
pub mod engine;
pub mod kana_table;
pub mod ngram;
pub mod platform;
pub mod scanmap;
pub mod types;
pub mod vk;
pub mod yab;

// Re-export PhysicalPos for ergonomic access from external crates and .yab integration.
pub use scanmap::PhysicalPos;
