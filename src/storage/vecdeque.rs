use std::{
    alloc::{alloc, dealloc, Layout},
    marker::PhantomData,
    ops::{Index, IndexMut, RangeBounds},
    ptr::copy_nonoverlapping,
    slice::from_raw_parts_mut,
};

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

impl<T: Sized> Default for VecDeque<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Sized> VecDeque<T> {
    #[inline]
    pub fn new() -> Self {
        Self::with_capacity(2)
    }
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0);
        assert!(capacity % 2 == 0);
        unsafe {
            let layout = Layout::array::<T>(capacity).expect("LAYOUT");
            let ptr = alloc(layout) as *mut T;
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
        unsafe { self.ptr.add(self.head).write(element) };
        self.head = wrap_index(self.head + 1, self.cap);
        self.len += 1;
        if self.is_full() {
            self.grow(1);
        }
    }
    #[inline]
    pub fn push_front(&mut self, element: T) {
        unsafe { self.ptr.add(self.tail).write(element) };
        self.tail = wrap_index(self.tail + self.cap - 1, self.cap);
        self.len += 1;
        if self.is_full() {
            self.grow(1);
        }
    }
    #[inline]
    pub fn pop_back(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        self.head = wrap_index(self.head + self.cap - 1, self.cap);
        self.len -= 1;
        unsafe { Some(self.ptr.add(self.head).read()) }
    }
    #[inline]
    pub fn pop_front(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        self.tail = wrap_index(self.tail + 1, self.cap);
        self.len -= 1;
        unsafe { Some(self.ptr.add(self.tail).read()) }
    }
    #[inline]
    pub fn clear(&mut self) {
        let mut index = self.tail;
        for _ in 0..self.len {
            index = wrap_index(index + 1, self.cap);
            unsafe { self.ptr.add(index).drop_in_place() };
        }
        self.len = 0;
        self.head = 0;
        self.tail = self.cap - 1;
    }
    #[inline]
    pub fn reserve(&mut self, capacity: usize) {
        if self.cap < capacity {
            self.grow(capacity - self.cap)
        }
    }
    #[inline]
    pub fn grow(&mut self, additional: usize) {
        let target = self.cap + additional;
        let old_layout = unsafe { Layout::array::<T>(self.cap).unwrap_unchecked() };
        let mut cap = self.cap;
        while cap < target {
            cap *= 2;
        }
        let new_layout = Layout::array::<T>(cap);

        let Ok(new_layout) = new_layout else {
            panic!("capacity overflow");
        };
        if usize::BITS < 64 && new_layout.size() > isize::MAX as usize {
            panic!("capacity overflow");
        }

        self.ptr = unsafe {
            let new_ptr = alloc(new_layout) as *mut T;
            let (front, back) = self.as_slices();
            copy_nonoverlapping(front.as_ptr(), new_ptr, front.len());
            copy_nonoverlapping(back.as_ptr(), new_ptr.add(front.len()), back.len());
            dealloc(self.ptr as *mut u8, old_layout);
            new_ptr
        };
        self.head = self.len;
        self.tail = cap - 1;
        self.cap = cap;
    }
    #[inline]
    pub fn as_slices(&mut self) -> (&mut [T], &mut [T]) {
        let tail = wrap_index(self.tail + 1, self.cap);
        let contiguous = tail < self.head || tail == 0;
        if contiguous {
            unsafe { (from_raw_parts_mut(self.ptr.add(tail), self.len), &mut []) }
        } else {
            unsafe {
                (
                    from_raw_parts_mut(self.ptr.add(tail), self.cap - self.tail - 1),
                    from_raw_parts_mut(self.ptr, self.head),
                )
            }
        }
    }
    #[inline]
    pub fn iter(&self) -> Iter<T> {
        Iter {
            remaining: self.len,
            index: self.tail,
            cap: self.cap,
            ring: self.ptr,
            _marker: PhantomData,
        }
    }
    #[inline]
    pub fn iter_mut(&self) -> IterMut<T> {
        IterMut {
            remaining: self.len,
            index: self.tail,
            cap: self.cap,
            ring: self.ptr,
            _marker: PhantomData,
        }
    }
    #[inline]
    pub fn drain<R>(&mut self, _range: R) -> Drain<T>
    where
        R: RangeBounds<usize>,
    {
        let drain = Drain {
            remaining: self.len,
            index: self.tail,
            cap: self.cap,
            ring: self.ptr,
            _marker: PhantomData,
        };
        self.len = 0;
        self.head = 0;
        self.tail = self.cap - 1;
        drain
    }
}

impl<T: Sized> Index<usize> for VecDeque<T> {
    type Output = T;
    fn index(&self, i: usize) -> &Self::Output {
        unsafe { &*self.ptr.add(wrap_index(self.tail + 1 + i, self.cap)) }
    }
}

impl<T: Sized> IndexMut<usize> for VecDeque<T> {
    fn index_mut(&mut self, i: usize) -> &mut T {
        unsafe { &mut *self.ptr.add(wrap_index(self.tail + 1 + i, self.cap)) }
    }
}

pub struct Iter<'a, T: 'a> {
    remaining: usize,
    index: usize,
    cap: usize,
    ring: *const T,
    _marker: PhantomData<&'a ()>,
}
pub struct IterMut<'a, T: 'a> {
    remaining: usize,
    index: usize,
    cap: usize,
    ring: *mut T,
    _marker: PhantomData<&'a ()>,
}
pub struct Drain<'a, T: 'a> {
    remaining: usize,
    index: usize,
    cap: usize,
    ring: *mut T,
    _marker: PhantomData<&'a ()>,
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
        Some(unsafe { &*self.ring.add(self.index) })
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
        Some(unsafe { &mut *self.ring.add(self.index) })
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
        Some(unsafe { self.ring.add(self.index).read() })
    }
}

#[cfg(test)]
macro_rules! assert_vec {
    ($v:ident: $($e:expr),* ; $cap:expr) => {
        assert_eq!($v.cap, $cap);
        let mut i = 0;
        $(
            assert_eq!($v[i], $e);
            i += 1 ;
        )*
        assert_eq!($v.len, i);
    };
}

#[test]
fn custom_vecdeque() {
    let mut v = VecDeque::with_capacity(4);
    v.push_back(1);
    v.push_back(2);
    v.push_back(3);
    v.push_back(4);
    assert_vec!(v: 1, 2, 3, 4; 8);
    let mut v = VecDeque::with_capacity(4);
    v.push_back(3);
    v.push_back(4);
    v.push_front(2);
    v.push_front(1);
    assert_vec!(v: 1, 2, 3, 4; 8);
    let mut v = VecDeque::with_capacity(4);
    v.push_back(2);
    v.push_front(1);
    v.reserve(5);
    assert_vec!(v: 1, 2; 8);
    let mut v = VecDeque::with_capacity(4);
    v.push_back(0);
    v.pop_front();
    v.push_back(1);
    v.push_back(2);
    v.push_back(3);
    v.push_back(4);
    assert_vec!(v: 1, 2, 3, 4; 8);
    let mut v = VecDeque::with_capacity(4);
    v.push_back(0);
    v.pop_front();
    v.push_back(1);
    v.push_back(2);
    v.reserve(5);
    assert_vec!(v: 1, 2; 8);
    let mut v = VecDeque::with_capacity(4);
    v.push_back(0);
    v.pop_front();
    v.push_back(1);
    v.push_back(2);
    v.push_back(3);
    v.reserve(5);
    assert_vec!(v: 1, 2, 3; 8);
}
