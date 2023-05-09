#[derive(Debug)]
pub struct VecDeque<T: Sized> {
    tail: usize,
    head: usize,
    cap: usize,
    len: usize,
    ptr: *mut T,
}

#[inline(always)]
fn wrap_index(index: usize, cap: usize) -> usize {
    index & (cap - 1)
}

impl<T: Sized> VecDeque<T> {
    #[inline]
    pub fn new() -> Self {
        Self::with_capacity(32)
    }
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0);
        unsafe {
            let layout = std::alloc::Layout::array::<T>(capacity).expect("LAYOUT");
            let ptr = std::alloc::alloc(layout) as *mut T;
            Self {
                tail: capacity - 1,
                head: 0,
                cap: capacity,
                len: 0,
                ptr,
            }
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len == self.cap
    }
    #[inline]
    pub fn push_back(&mut self, element: T) {
        unsafe { self.ptr.offset(self.head as isize).write(element) };
        self.head = wrap_index(self.head + 1, self.cap);
        self.len += 1;
        if self.is_full() {
            unreachable!();
        }
    }
    #[inline]
    pub fn push_front(&mut self, element: T) {
        unsafe { self.ptr.offset(self.tail as isize).write(element) };
        self.tail = wrap_index(self.tail + self.cap - 1, self.cap);
        self.len += 1;
        if self.is_full() {
            unreachable!();
        }
    }
    #[inline]
    pub fn pop_back(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        self.head = wrap_index(self.head + self.cap - 1, self.cap);
        self.len -= 1;
        unsafe { Some(self.ptr.offset(self.head as isize).read()) }
    }
    #[inline]
    pub fn pop_front(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        self.tail = wrap_index(self.tail + 1, self.cap);
        self.len -= 1;
        unsafe { Some(self.ptr.offset(self.tail as isize).read()) }
    }
    pub fn clear(&mut self) {
        let mut index = self.tail;
        for _ in 0..self.len {
            index = wrap_index(index + 1, self.cap);
            unsafe { self.ptr.offset(index as isize).drop_in_place() };
        }
        self.len = 0;
        self.head = 0;
        self.tail = self.cap - 1;
    }
    pub fn iter(&self) -> Iter<T> {
        // let ring = unsafe { std::slice::from_raw_parts(self.ptr, self.cap) };
        Iter {
            remaining: self.len,
            index: self.tail,
            cap: self.cap,
            ring: self.ptr,
            _a: std::marker::PhantomData,
        }
    }
    pub fn iter_mut(&self) -> IterMut<T> {
        // let ring = unsafe { std::slice::from_raw_parts_mut(self.ptr, self.cap) };
        IterMut {
            remaining: self.len,
            index: self.tail,
            cap: self.cap,
            ring: self.ptr,
            _a: std::marker::PhantomData,
        }
    }
    pub fn reserve(&mut self, _capacity: usize) {}
    pub fn drain<R>(&mut self, _range: R) -> Drain<T>
    where
        R: std::ops::RangeBounds<usize>,
    {
        let drain = Drain {
            remaining: self.len,
            index: self.tail,
            cap: self.cap,
            ring: self.ptr,
            _a: std::marker::PhantomData,
        };
        self.len = 0;
        self.head = 0;
        self.tail = self.cap - 1;
        drain
    }
}

pub struct Iter<'a, T: 'a> {
    remaining: usize,
    index: usize,
    cap: usize,
    ring: *const T,
    _a: std::marker::PhantomData<&'a ()>,
}
pub struct IterMut<'a, T: 'a> {
    remaining: usize,
    index: usize,
    cap: usize,
    ring: *mut T,
    _a: std::marker::PhantomData<&'a ()>,
}
pub struct Drain<'a, T: 'a> {
    remaining: usize,
    index: usize,
    cap: usize,
    ring: *mut T,
    _a: std::marker::PhantomData<&'a ()>,
}

impl<'a, T: Sized> IntoIterator for &'a VecDeque<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a, T: Sized> IntoIterator for &'a mut VecDeque<T> {
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<'a, T: Sized> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.index = wrap_index(self.index + 1, self.cap);
        Some(unsafe { self.ring.offset(self.index as isize).as_ref().unwrap() })
    }
}
impl<'a, T: Sized> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.index = wrap_index(self.index + 1, self.cap);
        Some(unsafe { self.ring.offset(self.index as isize).as_mut().unwrap() })
    }
}
impl<'a, T: Sized> Iterator for Drain<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.index = wrap_index(self.index + 1, self.cap);
        Some(unsafe { self.ring.offset(self.index as isize).read() })
    }
}
