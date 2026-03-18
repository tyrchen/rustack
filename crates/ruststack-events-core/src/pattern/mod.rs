//! EventBridge event pattern matching engine.
//!
//! Implements the full EventBridge pattern matching language including
//! all 10+ comparison operators and the `$or` logical combinator.

mod engine;
mod operators;
mod parser;
mod value;

pub use engine::matches;
pub use parser::{PatternParseError, parse_event_pattern};
pub use value::{
    AnythingButCondition, EventPattern, FieldMatcher, MatchCondition, NumericBound,
    NumericCondition, PatternNode,
};
