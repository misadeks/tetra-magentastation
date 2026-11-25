pub(crate) struct Queue<T, const N: usize> {
    items: [Option<T>; N],
    len: u16,
}

impl<T, const N: usize> Queue<T, N> {
    /// # Safety
    /// `out` must designate writable, aligned storage for Self, exclusively
    /// borrowed until initialization completes. No old value is dropped.
    pub unsafe fn init_at(out: *mut Self) {
        // SAFETY: The caller guarantees the complete Queue storage. Each array
        // slot is initialized exactly once; None does not require an initialized T.
        unsafe {
            let items = core::ptr::addr_of_mut!((*out).items).cast::<Option<T>>();
            for i in 0..N {
                items.add(i).write(None);
            }
            core::ptr::addr_of_mut!((*out).len).write(0);
        }
    }

    pub const fn new() -> Self {
        Self {
            items: [const { None }; N],
            len: 0,
        }
    }

    pub fn len(&self) -> u16 {
        self.len
    }

    pub fn free(&self) -> u16 {
        (N - usize::from(self.len)) as u16
    }

    pub fn push(&mut self, item: T) {
        // All callers reserve capacity before changing protocol state.
        debug_assert!(self.free() != 0);
        self.items[usize::from(self.len)] = Some(item);
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T> {
        self.remove(0)
    }

    pub fn remove(&mut self, index: u16) -> Option<T> {
        if index >= self.len {
            return None;
        }
        let index = usize::from(index);
        let result = self.items[index].take();
        for i in index + 1..usize::from(self.len) {
            self.items.swap(i - 1, i);
        }
        self.len -= 1;
        result
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items[..usize::from(self.len)].iter().flatten()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items[..usize::from(self.len)].iter_mut().flatten()
    }

    pub fn clear(&mut self) {
        for item in &mut self.items {
            *item = None;
        }
        self.len = 0;
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        let mut i = 0;
        while i < self.len {
            if self.items[usize::from(i)].as_ref().is_some_and(&mut keep) {
                i += 1;
            } else {
                self.remove(i);
            }
        }
    }
}
