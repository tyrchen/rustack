//! Bounded Unicode glob, independent of user regular expressions.

use super::Budget;
use crate::AslError;

#[derive(Clone)]
pub(super) struct Glob(Vec<Token>);
#[derive(Clone, Copy)]
enum Token {
    Literal(char),
    Star,
}

impl Glob {
    pub(super) fn compile(pattern: &str) -> Result<Self, AslError> {
        if pattern.len() > 4096 {
            return Err(AslError::InvalidPredicate);
        }
        let mut tokens = Vec::new();
        let mut chars = pattern.chars();
        while let Some(ch) = chars.next() {
            let token = match ch {
                '*' => {
                    if matches!(tokens.last(), Some(Token::Star)) {
                        continue;
                    }
                    Token::Star
                }
                '\\' => match chars.next() {
                    Some(ch @ ('*' | '\\')) => Token::Literal(ch),
                    _ => return Err(AslError::InvalidPredicate),
                },
                ch => Token::Literal(ch),
            };
            if tokens.len() >= 1024 {
                return Err(AslError::InvalidPredicate);
            }
            tokens.push(token);
        }
        Ok(Self(tokens))
    }

    pub(super) fn matches(&self, text: &str, budget: &mut Budget) -> Result<bool, AslError> {
        let mut position = 0_usize;
        let mut token = 0_usize;
        let mut star = None;
        while let Some(ch) = text.get(position..).and_then(|s| s.chars().next()) {
            budget.step()?;
            match self.0.get(token) {
                Some(Token::Literal(expected)) if *expected == ch => {
                    position = position
                        .checked_add(ch.len_utf8())
                        .ok_or(AslError::WorkLimit)?;
                    token = token.checked_add(1).ok_or(AslError::WorkLimit)?;
                }
                Some(Token::Star) => {
                    token = token.checked_add(1).ok_or(AslError::WorkLimit)?;
                    if token == self.0.len() {
                        return Ok(true);
                    }
                    star = Some((token, position));
                }
                _ => {
                    let Some((after, backup)) = star else {
                        return Ok(false);
                    };
                    let Some(next) = text.get(backup..).and_then(|s| s.chars().next()) else {
                        return Ok(false);
                    };
                    let advanced = backup
                        .checked_add(next.len_utf8())
                        .ok_or(AslError::WorkLimit)?;
                    position = advanced;
                    token = after;
                    star = Some((after, advanced));
                }
            }
        }
        while matches!(self.0.get(token), Some(Token::Star)) {
            budget.step()?;
            token = token.checked_add(1).ok_or(AslError::WorkLimit)?;
        }
        Ok(token == self.0.len())
    }
}
