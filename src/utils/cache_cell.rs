use std::cell::{Cell, UnsafeCell};

/// `CacheCell` is a structure that holds data that can be lazily evaluated and cached.
/// It provides the `.map_inner` on the guard object, which allows the inner value to be
/// mapped into a subset of the value.
///
/// It accomplishes similar stuff to RefCell, but with the extra `.map_inner` function.
pub struct CacheCell<Value> {
    data: UnsafeCell<Value>,
    borrowed: Cell<bool>,
}

impl<Value> CacheCell<Value> {
    /// Creates a new `CacheCell` instance.
    pub fn new(value: Value) -> Self {
        Self {
            data: UnsafeCell::new(value),
            borrowed: Cell::new(false),
        }
    }

    /// Updates the cache as long as it's not borrowed.
    #[inline(always)]
    pub fn update(&self, modify: impl FnOnce(&mut Value)) {
        if self.borrowed.get() {
            panic!("CacheCell is already borrowed!");
        }

        let mut value = unsafe { &mut *self.data.get() };

        // Modify value based on the closure passed.
        modify(value);
    }

    /// Fetches the cache value, tracking whether it's borrowed or not.
    /// If it's already borrowed, this will panic.
    ///
    /// The returned guard object allows calling `.map_inner` to map the inner value
    /// into a subset of the value.
    #[inline(always)]
    pub fn get<'a>(&'a self) -> CacheCellGuard<'a, Value> {
        if self.borrowed.get() {
            panic!("CacheCell is already borrowed!");
        }

        self.borrowed.set(true);

        // Return the guard object that allows access to the inner value.
        CacheCellGuard {
            borrowed: &self.borrowed,
            value: unsafe { &(*self.data.get()) },
        }
    }
}

pub struct CacheCellGuard<'a, Value: ?Sized> {
    borrowed: &'a Cell<bool>,
    value: &'a Value,
}

impl<'a, Value> CacheCellGuard<'a, Value> {
    #[inline(always)]
    pub fn map_inner<'b, NewValue: ?Sized>(
        self,
        map: impl FnOnce(&'a Value) -> &'b NewValue,
    ) -> CacheCellGuard<'b, NewValue>
    where
        'a: 'b,
    {
        CacheCellGuard {
            borrowed: self.borrowed,
            value: map(self.value),
        }
    }
}

impl<'a, Value: ?Sized> std::ops::Deref for CacheCellGuard<'a, Value> {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, Value: ?Sized> Drop for CacheCellGuard<'a, Value> {
    fn drop(&mut self) {
        self.borrowed.set(false);
    }
}
