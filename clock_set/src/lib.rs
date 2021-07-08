use std::cell::UnsafeCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
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

pub struct CSet<T: Hash + PartialEq> {
    size: AtomicUsize,
    head: UnsafeCell<LockNode<T>>,
}

impl<T: Hash + PartialEq> CSet<T> {
    pub fn new() -> Self {
        let tail = Box::into_raw(Box::new(LockNode::<T>::marker(u64::MAX)));
        let mut head = LockNode::marker(u64::MIN);
        // SAFETY: tail is a valid pointer as per the construction of Box
        head.next = unsafe { Some(NonNull::new_unchecked(tail)) };
        Self {
            size: AtomicUsize::new(0),
            head: UnsafeCell::new(head),
        }
    }

    pub fn size(&self) -> usize {
        self.size.load(SeqCst)
    }

    fn find(
        &self,
        key: u64,
    ) -> (
        (ManuallyDrop<MutexGuard<'_, ()>>, *mut LockNode<T>),
        (ManuallyDrop<MutexGuard<'_, ()>>, *mut LockNode<T>),
    ) {
        // SAFETY: We never cast the pointer to a mutable reference or shared reference.
        // All of the changes to the nodes are done explicitly when holding the locks
        let mut prev = self.head.get();
        // Safety we know the pointer was created using a box
        let mut prev_lock = ManuallyDrop::new(unsafe { (*prev).lock.lock().unwrap() });

        let mut curr = unsafe { (*prev).next }.as_ref().unwrap().as_ptr();
        // SAFETY: We know the pointer was created using a box
        let mut curr_lock = ManuallyDrop::new(unsafe { (*curr).lock.lock().unwrap() });

        loop {
            // SAFETY: Same as above
            if unsafe { (*curr).key } >= key {
                return((prev_lock, prev), (curr_lock, curr));
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

    pub fn contains(_like: &T) {}
    pub fn remove(_like: &T) {}

    pub fn insert(&self, val: T) -> bool {
        let key = calc_hash(&val);
        let mut to_insert = LockNode::new(val);

        let ((prev_lock, prev), (curr_lock, curr)) = self.find(key);
        if unsafe { (*prev).key } == key {
            // The value is already in the set
            let _ = ManuallyDrop::into_inner(prev_lock);
            let _ = ManuallyDrop::into_inner(curr_lock);
            return false;
        }
        // The value isn't in the set, but we add it inbetween the two nodes that we got
        to_insert.next = Some(unsafe { NonNull::new_unchecked(curr) });
        let to_insert = Box::into_raw(Box::new(to_insert));

        // SAFETY: The casting from *mut to &mut only ever happens once, here.
        // We are guaranteed that no other thread makes this operation at the same time as us,
        // as we are the ones holding these two locks at the same time, so we are the only
        // ones using a &mut to this specific node, so there is no aliasing going on.
        let prev_mut = unsafe { &mut (*prev).next };
        // Put our node in the middle
        *prev_mut = Some(unsafe { NonNull::new_unchecked(to_insert) });

        let _ = ManuallyDrop::into_inner(prev_lock);
        let _ = ManuallyDrop::into_inner(curr_lock);
        self.size.fetch_add(1, SeqCst);

        true
    }
}
