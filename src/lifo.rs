//! See http://www.open-std.org/jtc1/sc22/wg21/docs/papers/2019/p1726r1.pdf This
//! implementation doesn't aim for idiomatic rust, rather for being obviously
//! the same.

use std::{
    hint::spin_loop,
    marker::PhantomData,
    sync::atomic::{AtomicPtr, Ordering},
};

/// AtomicOptionBox-alike but tailored for this algorithm. Conceptually this
/// owns, or might own a T, and allows interior mutability.
struct AtomicOptionBox<T> {
    ptr: AtomicPtr<T>,
    _marker: PhantomData<T>,
}

impl<T> AtomicOptionBox<T> {
    /// *new -> self, self->*current
    pub fn spin_swap(
        &self,
        current: *mut AtomicOptionBox<T>,
        new: *mut T,
        success: Ordering,
        failure: Ordering,
    ) {
        loop {
            match unsafe {
                self.ptr
                    .compare_exchange_weak(*(*current).ptr.get_mut(), new, success, failure)
            } {
                Ok(_) => break,
                Err(x) => {
                    unsafe {
                        *(*current).ptr.get_mut() = x;
                    }
                    spin_loop();
                }
            }
        }
    }

    pub fn is_none(&self) -> bool {
        // Only consider the bits of the pointer
        let ptr = self.ptr.load(Ordering::Relaxed);
        ptr.is_null()
    }

    pub fn take(&self, order: Ordering) -> AtomicOptionBox<T> {
        let ptr: *mut T = std::ptr::null_mut();
        let p = self.ptr.swap(ptr, order);
        let p = AtomicPtr::new(p);
        Self {
            ptr: p,
            ..Default::default()
        }
    }

    pub fn unwrap(&mut self, ordering: Ordering) -> T {
        let ptr = self.ptr.load(ordering);
        if ptr.is_null() {
            panic!("unwrap called on None AtomicOptionBox");
        } else {
            *(unsafe { Box::from_raw(ptr) })
        }
    }
}

impl<T> Default for AtomicOptionBox<T> {
    fn default() -> Self {
        let ptr = AtomicPtr::new(std::ptr::null_mut());
        Self {
            ptr,
            ..Default::default()
        }
    }
}

struct Node<T> {
    val: T,
    /// One equivalent to Node *next in C++: Box is a zero-sized heap ownership
    /// abstraction that doesn't have a null equivalent; and Option gives the
    /// nullability aspect.
    pub next: AtomicOptionBox<Node<T>>,
}

impl<T> Node<T> {
    fn new(val: T) -> Self {
        Self {
            val,
            next: Default::default(),
        }
    }

    fn into_inner(self) -> (AtomicOptionBox<Node<T>>, T) {
        (self.next, self.val)
    }
}

#[derive(Default)]
pub struct LifoPush<T> {
    top: AtomicOptionBox<Node<T>>,
}

impl<T> LifoPush<T> {
    pub fn list_empty(&self) -> bool {
        self.top.is_none()
    }

    pub fn push(&self, val: T) {
        let mut newnode = Box::new(Node::new(val));
        let current: *mut AtomicOptionBox<Node<T>> = &mut newnode.next;
        let new: *mut Node<T> = Box::into_raw(newnode);
        // Release so that the Acquire in list_pop_all can see the contents of newnode.
        self.top
            .spin_swap(current, new, Ordering::Release, Ordering::Relaxed);
    }

    pub fn pop_all<F>(&self, mut f: F)
    where
        F: FnMut(T),
    {
        let mut head = self.top.take(Ordering::Relaxed);
        // Readers may be at any position
        while !head.is_none() {
            // Acquire all memory writes as we need to see the contents of the item
            let (next, item) = head.unwrap(Ordering::Acquire).into_inner();
            f(item);
            head = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Instant};

    use crossbeam_utils::thread::scope;

    use super::LifoPush;

    fn timed<F: Fn()>(f: F) {
        let now = Instant::now();
        while now.elapsed().as_secs() < 10 {
            println!("{:?}", now.elapsed().as_secs());
            f()
        }
    }

    #[test]
    fn paper_scenario() {
        // Invalid -language-level- but valid assembly level from the paper:
        // T1: load top -> var
        // T2: null->top, top-> processed and free
        // T1: var -> newnode.next
        // T2: alloc newnode1 @ old top addr and push to top
        // T1: CXW : newnode -> top
        // T2: thread_pop_all; reads newnode then newnode1 then null.

        timed(|| {
            let list: LifoPush<i64> = super::LifoPush::default();
            // list.push(45);
            // scope(|s| {
            //     s.spawn(|_| {
            //         list.push(67);
            //     });

            //     s.spawn(|_| {
            //         list.pop_all(|_num| {});
            //         list.push(89);
            //         let mut acc = 0;
            //         let acc_ref = &mut acc;
            //         list.pop_all(|num| *acc_ref += num);
            //         assert!(acc == 134 || acc == 201);
            //     });
            // })
            // .unwrap();
        });
    }
}
