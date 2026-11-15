//! Memory for sounds: blocks of one size, never anything else.
//!
//! A program that runs for weeks must not pile up holes in its memory, and loading and dropping sounds of every length
//! would do exactly that. So every sound is made of blocks of the same size, taken from a pool and given back to it:
//! nothing is allocated or freed while sounds come and go, only handed around. The pool grows one block at a time,
//! when asked and up to its limit (a soundboard nobody uses reserves nothing), and gives everything back at once when
//! the last soundboard of the running graph goes away.
//!
//! A block that is being read is held by an `Arc`: it is only taken back (and rewritten) once nobody holds it, which is
//! checked, never assumed; this needs no unsafe code.

use super::BLOCK_SAMPLES;
use std::sync::Arc;

/// A piece of a sound: `BLOCK_SAMPLES` samples (32 KB) of which `len` are used.
#[derive(Debug)]
pub struct Block {
    pub data: Box<[i16]>,
    /// Samples in use (less than the size only at the end of a sound).
    pub len: usize,
    /// The last block of its sound (a sound that is read by segments says when it ends).
    pub last: bool,
}

pub type BlockRef = Arc<Block>;

impl Block {
    fn new() -> Block {
        Block {
            data: vec![0i16; BLOCK_SAMPLES].into_boxed_slice(),
            len: 0,
            last: false,
        }
    }
}

/// The blocks, their limit, and the ones that are free.
#[derive(Debug)]
pub struct BlockPool {
    limit: usize,
    allocated: usize,
    free: Vec<BlockRef>,
}

impl BlockPool {
    /// A pool that will never hold more than `limit_bytes` (rounded down to whole blocks, at least one).
    pub fn new(limit_bytes: u64) -> BlockPool {
        let block_bytes = (BLOCK_SAMPLES * 2) as u64;
        BlockPool {
            limit: ((limit_bytes / block_bytes) as usize).max(1),
            allocated: 0,
            free: Vec::new(),
        }
    }

    /// The most blocks the pool will ever hold.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Blocks made so far (free or in use).
    pub fn allocated(&self) -> usize {
        self.allocated
    }

    /// Blocks in use right now.
    pub fn in_use(&self) -> usize {
        self.allocated - self.free.len()
    }

    /// Blocks that can be taken without evicting anything: free ones, and the ones the pool may still make.
    pub fn available(&self) -> usize {
        self.free.len() + (self.limit - self.allocated)
    }

    /// A block nobody else holds (empty), or `None` when the limit is reached and none is free.
    pub fn take(&mut self) -> Option<BlockRef> {
        if let Some(mut b) = self.free.pop() {
            let inner = Arc::get_mut(&mut b).expect("a free block is held by nobody");
            inner.len = 0;
            inner.last = false;
            return Some(b);
        }
        if self.allocated < self.limit {
            self.allocated += 1;
            return Some(Arc::new(Block::new()));
        }
        None
    }

    /// Takes a block back. A block somebody still holds is not taken back: it stays counted as in use until the pool
    /// is dropped (this does not happen in this program; it would only waste a block).
    pub fn give(&mut self, block: BlockRef) {
        if Arc::strong_count(&block) == 1 {
            self.free.push(block);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_grows_one_block_at_a_time_up_to_its_limit_and_reuses_what_is_given_back() {
        let mut pool = BlockPool::new(3 * 32 * 1024);
        assert_eq!(
            (pool.limit(), pool.allocated(), pool.available()),
            (3, 0, 3)
        );
        let a = pool.take().unwrap();
        let b = pool.take().unwrap();
        assert_eq!(
            (pool.allocated(), pool.in_use(), pool.available()),
            (2, 2, 1)
        );
        let c = pool.take().unwrap();
        assert!(pool.take().is_none(), "the limit is the limit");
        pool.give(a);
        assert_eq!(pool.available(), 1);
        // the block that comes back is the one that was given, not a new one
        let again = pool.take().unwrap();
        assert_eq!(pool.allocated(), 3);
        pool.give(b);
        pool.give(c);
        pool.give(again);
        assert_eq!(
            (pool.in_use(), pool.allocated()),
            (0, 3),
            "nothing is ever freed while the pool lives"
        );
    }

    #[test]
    fn a_block_somebody_still_reads_is_not_taken_back() {
        let mut pool = BlockPool::new(2 * 32 * 1024);
        let a = pool.take().unwrap();
        let reader = a.clone();
        pool.give(a); // still held by `reader`
        assert_eq!(pool.in_use(), 2 - pool.available(), "counted, not reused");
        assert_eq!(pool.available(), 1);
        drop(reader);
    }

    #[test]
    fn a_pool_has_at_least_one_block() {
        assert_eq!(BlockPool::new(0).limit(), 1);
    }
}
