//! Stub — filled in by Task 8.

use crate::ai::Category;
use crate::types::PriorityScore;

#[derive(Debug, Clone)]
pub struct Classification {
    pub priority: PriorityScore,
    pub category: Category,
    pub reasoning: String,
}

pub struct Classifier;
