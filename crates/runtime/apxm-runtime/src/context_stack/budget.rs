//! Token budget accounting for ContextStack frame assembly.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetAllocator {
    total: usize,
    used: usize,
}

impl BudgetAllocator {
    pub fn new(total: usize) -> Self {
        Self { total, used: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.total.saturating_sub(self.used)
    }

    pub fn allocate(&mut self, requested: usize) -> usize {
        let allocated = requested.min(self.remaining());
        self.used = self.used.saturating_add(allocated);
        allocated
    }

    pub fn is_exhausted(&self) -> bool {
        self.used >= self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_allocation_saturates() {
        let mut allocator = BudgetAllocator::new(100);

        assert_eq!(allocator.allocate(30), 30);
        assert_eq!(allocator.remaining(), 70);
        assert_eq!(allocator.allocate(80), 70);
        assert!(allocator.is_exhausted());
    }
}
