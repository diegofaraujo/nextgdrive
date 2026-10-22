/// Storage usage of a Google account, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuota {
    pub usage: u64,
    /// `None` when the account has unlimited storage.
    pub limit: Option<u64>,
}

impl StorageQuota {
    pub fn available(&self) -> Option<u64> {
        self.limit.map(|limit| limit.saturating_sub(self.usage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_space() {
        let quota = |usage, limit| StorageQuota { usage, limit };
        assert_eq!(quota(3, Some(10)).available(), Some(7));
        assert_eq!(quota(12, Some(10)).available(), Some(0));
        assert_eq!(quota(3, None).available(), None);
    }
}
