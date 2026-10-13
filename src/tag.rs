use serde::{Serialize, Deserialize};
use chrono::{DateTime, Duration, Utc};
use anyhow::{anyhow, Result};
use std::cmp::Ordering;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tag {
    // The name is a tree structure, e.g. "Technology > Artificial Intelligence"
    // The parent and children are separated by "/"
    // Example: "tech/ai", "math/affine", "tech/ai/machine-learning", "math/affine/linear-algebra"
    pub name: String,
}

impl Tag {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Time {
    Time(DateTime<Utc>),
    None,      // Can only be used as start time
    Now,       // Can only be used as end time
    Yesterday,
    LastWeek,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TimeRange {
    pub start: Time,
    pub end: Time,
}

impl PartialEq for Time {
    fn eq(&self, other: &Self) -> bool {
        self.to_datetime() == other.to_datetime()
    }
}

impl PartialOrd for Time {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.to_datetime().partial_cmp(&other.to_datetime())
    }
}

impl Time {
    pub fn to_datetime(&self) -> Option<DateTime<Utc>> {
        match self {
            Time::Time(dt) => Some(*dt),
            Time::Now => Some(Utc::now()),
            Time::Yesterday => Some(Utc::now() - Duration::days(1)),
            Time::LastWeek => Some(Utc::now() - Duration::days(7)),
            Time::None => None,
        }
    }
}

impl TimeRange { 
    // Parse a time range string like "2024-01-01 ~ 2024-01-31", the format is standard rfc3339
    pub fn new(time_str: &str) -> Result<Self, anyhow::Error> {
        let parts: Vec<&str> = time_str.split('~').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid time range format. Expected 'start ~ end' but got: {}", time_str));
        }
        
        let start_str = parts[0].trim();
        let end_str = parts[1].trim();

        let start_time = if start_str.is_empty() {
            Time::None
        } else if start_str.eq_ignore_ascii_case("yesterday") {
            // Get yesterday's date
            Time::Yesterday
        } else if start_str.eq_ignore_ascii_case("last-week") {
            Time::LastWeek
        } else {
            match DateTime::parse_from_rfc3339(start_str) {
                Ok(dt) => Time::Time(dt.with_timezone(&Utc)),
                Err(e) => return Err(anyhow!("Invalid start date '{}': {}", start_str, e))
            }
        };

        let end_time = if end_str.is_empty() {
            Time::Now
        } else if end_str.eq_ignore_ascii_case("yesterday") {
            Time::Yesterday
        } else if end_str.eq_ignore_ascii_case("last-week") {
            Time::LastWeek
        } else {
            match DateTime::parse_from_rfc3339(end_str) {
                Ok(dt) => Time::Time(dt.with_timezone(&Utc)),
                Err(e) => return Err(anyhow!("Invalid end date '{}': {}", end_str, e))
            }
        };

        Ok(Self { start: start_time, end: end_time })
    }

}
