/// This is just a proof of concept example
/// of a concurrent linked list implementation
/// that can be used as a set (membership is 
/// validated via "keys", which MUST be unique). 
/// Alternatively, we could also check object equality instead of only 
/// the keys, but that detail is relatively unimportant compared to the concept.
/// The key thing here is that we are totally circumventing the borrow-checker 
/// to lock in a "hand over hand" fashion. We make sure that all of the
/// accesses to the pointers are synchronized via the locks, but the compiler 
/// has no way of understanding that (hence the use of raw pointers).
/// This is one of the examples where unsafe is necessary. The compiler cannot
/// tell that the very specific lock ordering we are doing guarantees that all accesses
/// are synchronized. The Mutex<()> is just a blank mutex for synchronization.
/// We could have used Mutex<LockNode<>>, but then we would have had to also wrap it
/// in an Arc, which seems unnecessary.
///
/// Linked lists are notorious for being awkward in Rust, and this is just an implementation
/// to experiment with unsafe and the kinds of things we can do with it.
/// Possibly hardest part of unsafe Rust is creating references from pointers,
/// to make sure we do not violate any of the aliasing rules. There is great care
/// in here to prevent that, as references are created ONLY when both locks are held
/// (i.e no other thread could be possibly doing the same thing at the same time as us).
use std::cell::UnsafeCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Mutex, MutexGuard};

struct LockNode<T: Hash + PartialEq> {
    key: u64,
    val: Option<T>,
    next: Option<NonNull<LockNode<T>>>,
    lock: Mutex<()>,
}

impl<T: Hash + PartialEq> LockNode<T> {
    fn new(val: T) -> Self {
        let key = calc_hash(&val);
        // Known limitation. We use 0 as a marker node for the head.
        // We could get around this using a signed type but Hasher uses unsigned types,
        // so this is a bit unlucky
        if key == 0 {
            panic!("Hashcodes of your value must always be non-zero");
        }
        LockNode {
            key,
            val: Some(val),
            next: None,
            lock: Mutex::new(()),
        }
    }

    fn new_with_key(val: T, key: u64) -> Self {
        if key == 0 {
            panic!("Hashcodes of your value must always be non-zero");
        }
        LockNode {
            key,
            val: Some(val),
            next: None,
            lock: Mutex::new(()),
        }
    }

    fn marker(key: u64) -> Self {
        LockNode {
            key,
            val: None,
            next: None,
            lock: Mutex::new(()),
        }
    }

    fn new_with_next(val: T, next: *mut LockNode<T>) -> Self {
        let key = calc_hash(&val);
        // SAFETY: We only ever pass valid pointers to this function. It is the
        // caller's responsibility that "next" is well-aligned and points to valid memory and is
        // non-null
        LockNode {
            key,
            val: Some(val),
            next: unsafe { Some(NonNull::new_unchecked(next)) },
            lock: Mutex::new(()),
        }
    }

    fn key(&self) -> u64 {
        self.key
    }
}
fn calc_hash<T: Hash>(t: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    t.hash(&mut hasher);
    hasher.finish()
}

unsafe impl<T: Hash + PartialEq> Send for CSet<T> {}
unsafe impl<T: Hash + PartialEq> Sync for CSet<T> {}

pub struct CSet<T: Hash + PartialEq> {
    size: AtomicUsize,
    head: UnsafeCell<LockNode<T>>,
}

impl<T: Hash + PartialEq> Drop for CSet<T> {
    fn drop(&mut self) {
        let head = self.head.get();
        // SAFETY: All pointers are from leaked valid boxes
        let mut next = unsafe { (*head).next };
        loop {
            if let Some(node_ptr) = next {
                // SAFETY: All pointers are from leaked valid boxes
                unsafe {
                    next = node_ptr.as_ref().next;
                    let _ = Box::from_raw(node_ptr.as_ptr());
                };
            } else {
                break;
            }
        }
    }
}

impl<T: Hash + PartialEq> CSet<T> {
    pub fn new() -> Self {
        let tail = Box::into_raw(Box::new(LockNode::<T>::marker(u64::MAX)));
        let mut head = LockNode::marker(u64::MIN);
        // SAFETY: All pointers are from leaked valid boxes
        head.next = unsafe { Some(NonNull::new_unchecked(tail)) };
        Self {
            size: AtomicUsize::new(0),
            head: UnsafeCell::new(head),
        }
    }


    pub fn print<F>(&mut self, printer: F)
    where
        F: Fn(&T),
    {
        let head = self.head.get();
        // SAFETY: All pointers are from leaked valid boxes
        let mut next = unsafe { (*head).next };
        loop {
            if let Some(node_ptr) = next {
                // SAFETY: All pointers are from leaked valid boxes
                unsafe {
                    if let Some(v) = &(*node_ptr.as_ptr()).val {
                        printer(v);
                    }
                    next = node_ptr.as_ref().next;
                };
            } else {
                println!();
                break;
            }
        }
    }

    pub fn size(&self) -> usize {
        // We are not using the atomic for any inter-thread communication,
        // just using it as a counter, so Relaxed ordering will do
        self.size.load(Relaxed)
    }

    fn find(
        &self,
        key: u64,
    ) -> (
        (ManuallyDrop<MutexGuard<'_, ()>>, *mut LockNode<T>),
        (ManuallyDrop<MutexGuard<'_, ()>>, *mut LockNode<T>),
    ) {
        // SAFETY: We never cast the pointer to a mutable reference or shared reference.
        // All of the changes to the nodes are done explicitly when holding the locks,
        // and we know the very first node is always the head marker node
        let mut prev = self.head.get();
        // SAFETY: All pointers are from leaked valid boxes
        let mut prev_lock = ManuallyDrop::new(unsafe { (*prev).lock.lock().unwrap() });

        // SAFETY: All pointers are from leaked valid boxes
        let mut curr = unsafe { (*prev).next }.as_ref().unwrap().as_ptr();
        // SAFETY: All pointers are from leaked valid boxes
        let mut curr_lock = ManuallyDrop::new(unsafe { (*curr).lock.lock().unwrap() });

        loop {
            // SAFETY: All pointers are from leaked valid boxes
            if unsafe { (*curr).key } >= key {
                return ((prev_lock, prev), (curr_lock, curr));
            }
            // Release the first lock we are holding, while keeping the second one
            let _ = ManuallyDrop::into_inner(prev_lock);
            prev = curr;
            prev_lock = curr_lock;
            // SAFETY: Same as above
            let next_ptr = unsafe { (*prev).next.unwrap().as_ptr() };
            curr_lock = ManuallyDrop::new(unsafe { (*next_ptr).lock.lock().unwrap() });
            curr = next_ptr;
        }
    }

    pub fn contains_with_key(&self, key: u64) -> bool {
        let ((prev_lock, _), (curr_lock, curr)) = self.find(key);
        // SAFETY: All pointers are from leaked valid boxes
        let is_inside = unsafe { (*curr).key } == key;
        let _ = ManuallyDrop::into_inner(prev_lock);
        let _ = ManuallyDrop::into_inner(curr_lock);
        is_inside
    }

    pub fn contains(&self, like: &T) -> bool {
        self.contains_with_key(calc_hash(like))
    }

    pub fn remove(&self, like: &T) -> bool {
        self.remove_with_key(calc_hash(like))
    }
    pub fn remove_with_key(&self, key: u64) -> bool {
        let ((prev_lock, prev), (curr_lock, curr)) = self.find(key);
        // SAFETY: All pointers are from leaked valid boxes
        if unsafe { (*curr).key } > key {
            // The item isn't in the set
            let _ = ManuallyDrop::into_inner(prev_lock);
            let _ = ManuallyDrop::into_inner(curr_lock);
            return false;
        }
        // The node is in the set, delete it

        // SAFETY: All pointers are from leaked valid boxes
        unsafe {
            let next_next = (*curr).next.unwrap().as_ptr();
            (*prev).next = Some(NonNull::new_unchecked(next_next));
        }

        // The decrement is before the release of the locks for the same reason as
        // mentioned in insert_with_key()
        self.size.fetch_sub(1, Relaxed);
        let _ = ManuallyDrop::into_inner(prev_lock);
        let _ = ManuallyDrop::into_inner(curr_lock);
        // SAFETY: curr is no longer needed, and we never gave any references to it
        // so it is safe to free it. It is mandatory as we do this here,
        // as curr_lock is a guard from "curr", so doing this in reverse order
        // would use-after-free
        let _ = unsafe { Box::from_raw(curr) };

        true
    }

    pub fn insert_with_key(&self, val: T, key: u64) -> bool {
        let mut to_insert = LockNode::new_with_key(val, key);

        let ((prev_lock, prev), (curr_lock, curr)) = self.find(key);
        // SAFETY: All pointers are from leaked valid boxes
        if unsafe { (*curr).key } == key {
            // The value is already in the set
            let _ = ManuallyDrop::into_inner(prev_lock);
            let _ = ManuallyDrop::into_inner(curr_lock);
            return false;
        }
        // The value isn't in the set, but we add it inbetween the two nodes that we got
        // SAFETY: All pointers are from leaked valid boxes
        to_insert.next = Some(unsafe { NonNull::new_unchecked(curr) });
        let to_insert = Box::into_raw(Box::new(to_insert));

        // SAFETY: The casting from *mut to &mut only ever happens once, here.
        // We are guaranteed that no other thread makes this operation at the same time as us,
        // as we are the ones holding these two locks at the same time, so we are the only
        // ones using a &mut to this specific node. More specifically, we are the only thread
        // creating a reference (mutable or not), because we only do this once we hold both nodes'
        // locks
        let prev_next_mut = unsafe { &mut (*prev).next };
        // Put our node in the middle
        // SAFETY: All pointers are from leaked valid boxes
        *prev_next_mut = Some(unsafe { NonNull::new_unchecked(to_insert) });

        // We are not using the atomic for any inter-thread communication,
        // just using it as a counter, so Relaxed ordering will do here.
        // We put the increment here, as it could be the very unlucky case that 
        // someone deletes the node we just added right after the lock release before the increment
        // which would be inconsistent, so it must happen before the release of the locks
        self.size.fetch_add(1, Relaxed);
        let _ = ManuallyDrop::into_inner(prev_lock);
        let _ = ManuallyDrop::into_inner(curr_lock);

        true
    }

    pub fn insert(&self, val: T) -> bool {
        let key = calc_hash(&val);
        self.insert_with_key(val, key)
    }
}
