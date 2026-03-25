//! Statistics types for NVS storage analysis.
//!
//! These types provide information about page and entry usage within
//! an NVS partition.

use alloc::vec::Vec;

/// Overall statistics for an NVS partition.
#[derive(Debug, Clone, PartialEq)]
pub struct NvsStatistics {
    pub pages: PageStatistics,
    pub entries_per_page: Vec<EntryStatistics>,
    pub entries_overall: EntryStatistics,
}

/// Statistics about page states in the partition.
#[derive(Debug, Clone, PartialEq)]
pub struct PageStatistics {
    pub empty: u16,
    pub active: u16,
    pub full: u16,
    pub erasing: u16,
    pub corrupted: u16,
}

/// Statistics about entry states within pages.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryStatistics {
    pub empty: u32,
    pub written: u32,
    pub erased: u32,
    pub illegal: u32,
}
