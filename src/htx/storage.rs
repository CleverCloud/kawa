use std::{cmp::min, io, ptr};

/// HtxBuffer is a pseudo ring buffer specifically designed to store data being parsed
/// ```
/// buffer        start   half     head  end   len
/// v             v       v         v     v     v
/// [             ████████:██████████░░░░░░     ]
/// <-------------------------------------------> buffer()        | capacity()
/// <------------------------------------->       used()          | end
///                                        <----> space()         | available_space()
///               <----------------------->       data()          | available_data()
///                                  <---->       unparsed_data() |
/// ```
/// `head` must be comprised between `start` and `end` and delimit parsed data from unparsed data.
/// The buffer is filled from `end` up to `buffer.len()`.
/// Data is assumed to be processed from left to right.
/// When data from the begining of the buffer can be discarded, `start` advances.
/// When `start` overshoot half the length of the buffer, it means half the buffer is unsued.
/// ```
/// buffer             half  start  head  end   len
/// v                     v  v      v     v     v
/// [                     :  ████████░░░░░░     ]
/// ```
/// At that point the remaining data of the buffer should be shifted.
/// Shifting the buffer memmoved the available data back at the begining of the buffer.
/// ```
/// buffer
/// start   head  end     half                  len
/// v       v     v       v                     v
/// [████████░░░░░░       :                     ]
/// ```
/// It is also recommended to shift an empty buffer if `start` is not 0.
/// ```
/// buffer   start/end    half                  len
/// v        v            v                     v
/// [        |            :                     ]
/// ```
pub struct HtxBuffer<'a> {
    pub start: usize,
    pub head: usize,
    pub end: usize,
    pub buffer: &'a mut [u8],
}

#[allow(dead_code)]
impl<'a> HtxBuffer<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            start: 0,
            head: 0,
            end: 0,
            buffer,
        }
    }

    pub fn meter(&self, half: usize) -> String {
        let size = half * 2 + 1;
        let len = self.capacity();
        (0..size + 2)
            .map(|i| {
                if i == 0 {
                    '['
                } else if i - 1 == half {
                    ':'
                } else if i - 1 < (self.start * size / len) {
                    ' '
                } else if i - 1 < (self.head * size / len) {
                    '█'
                } else if i - 1 < (self.end * size / len) {
                    '░'
                } else if i - 1 < size {
                    ' '
                } else {
                    ']'
                }
            })
            .collect()
    }

    pub fn available_data(&self) -> usize {
        self.end - self.start
    }

    pub fn available_space(&self) -> usize {
        self.capacity() - self.end
    }

    pub const fn capacity(&self) -> usize {
        self.buffer.len()
    }

    pub fn empty(&self) -> bool {
        self.start == self.end
    }

    pub fn consume(&mut self, count: usize) -> usize {
        let count = min(count, self.available_data());
        self.start += count;
        count
    }

    pub fn fill(&mut self, count: usize) -> usize {
        let count = min(count, self.available_space());
        self.end += count;
        count
    }

    pub fn should_shift(&self) -> bool {
        self.start > self.capacity() / 2 || (self.start > 0 && self.empty())
    }

    pub fn clear(&mut self) {
        self.start = 0;
        self.head = 0;
        self.end = 0;
    }

    pub fn buffer(&self) -> &[u8] {
        self.buffer
    }

    pub fn data(&self) -> &[u8] {
        let range = self.start..self.end;
        &self.buffer[range]
    }

    pub fn unparsed_data(&self) -> &[u8] {
        let range = self.head..self.end;
        &self.buffer[range]
    }

    pub fn space(&mut self) -> &mut [u8] {
        let range = self.end..self.capacity();
        &mut self.buffer[range]
    }

    pub fn used(&mut self) -> &[u8] {
        let range = ..self.end;
        &mut self.buffer[range]
    }

    pub fn shift(&mut self) -> usize {
        let start = self.start;
        let end = self.end;
        if start > 0 {
            unsafe {
                let len = end - start;
                ptr::copy(
                    self.buffer[start..end].as_ptr(),
                    self.buffer[..len].as_mut_ptr(),
                    len,
                );
                self.start = 0;
                self.head -= start;
                self.end = len;
            }
        }
        start
    }
}

impl io::Write for HtxBuffer<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.space().write(buf) {
            Ok(size) => {
                self.fill(size);
                Ok(size)
            }
            err => err,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Read for HtxBuffer<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = min(self.available_data(), buf.len());
        unsafe {
            ptr::copy(
                self.buffer[self.start..self.start + len].as_ptr(),
                buf.as_mut_ptr(),
                len,
            );
            self.start += len;
        }
        Ok(len)
    }
}
