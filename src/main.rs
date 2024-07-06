use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use std::cell::UnsafeCell;
use std::thread::spawn;

// Never ever use spin locks :), this implemetation is experimental
// Why?, read this - https://matklad.github.io/2020/01/02/spinlocks-considered-harmful.html

const LOCKED: bool = true;
const UNLOCKED: bool = false;

struct Mutex<T> {
    locked: AtomicBool,
    v: UnsafeCell<T>,
}

unsafe impl<T> Sync for Mutex<T> where T: Send {}

impl<T> Mutex<T> {
    pub fn new(t: T) -> Self {
        Self {
            locked: AtomicBool::new(UNLOCKED),
            v: UnsafeCell::new(t),
        }
    }
    fn with_lock<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self
            .locked
            .compare_exchange_weak(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // MESI protocol: stay in S when locked
            // Prevents threads from frequntly executing compare_exchange_weak which
            // requires exclusive access thereby leading to high contention
            while self.locked.load(Ordering::Relaxed) == LOCKED {}

            // Why compare_exchange_weak?
            // x86: CAS (Compare and Swap)
            // ARM: LDREX STREX - Load, Link and Store conditional

            // LDREX takes exclusive ownership of the value (to read or modify)
            // STREX stores the value only if there isn't another exclusive
            // ownership

            // - compare_exchange: impl using a loop of LDREX and STREX
            //  while LDREX is true
            //     while current state has changed e.g to UNLOCKED // ensure current state has changed to UNLOCK otherwise fail to execute STREX
            //                  STREX the new value
            // (simply means don't change this value if other threads are using it)
            // the inner loop ensure state is not changed before STREX is executed so
            // we ensure we are within the thread
            // This is cheap but costly in terms of regsitry pressure

            // - compare_exchange_weak: This option is offered so that instead of creating nested loop
            // for ARM, we can use compare_exchange_weak which is both compatible with ARM and x64, it also
            // supports spurious failure meaning anything that prevents swap to fail including previous state
            // not changed e.g from LOCKED to UNLOCKED or other reasons
        }
        self.locked.store(LOCKED, Ordering::Relaxed);
        let ret = f(unsafe { &mut *self.v.get() });
        self.locked.store(UNLOCKED, Ordering::Release);
        ret
    }
}

// The way to think about Release and Aqcuire is that acquire-release pair establishes a happens
// before relationship between the thread that previously released the lock and
// the next thread that takes the lock

// Before-After relationship states that what happens before the thing that triggered stored also
// happend before what happens after load, think of this as a bar like this

// a + 50  ---- 2 Thread B wants to aquire the lock and add 50.
//
// ==========================LOAD `a` (acquire) ================================= Thread A
//...
//... 
//... other instructions
//... a + 10 ----  1
//...
//... end
//===========================STORE `a` (release) ================================

// Since Thread A mutex allows releases only after execution, the next thread that will aquire the lock will see the
// updated value

fn main() {
    use std::sync::atomic::AtomicUsize;
    let x: &'static _ = Box::leak(Box::new(AtomicBool::new(false)));
    let y: &'static _ = Box::leak(Box::new(AtomicBool::new(false)));
    let z: &'static _ = Box::leak(Box::new(AtomicUsize::new(0)));

    let _tx = spawn(move || {
        x.store(true, Ordering::Release);
    });

    let _ty = spawn(move || {
        y.store(true, Ordering::Release);
    });

    let t1 = spawn(move || {
        while !x.load(Ordering::Acquire) {}
        if y.load(Ordering::Acquire) {
            z.fetch_add(1, Ordering::Relaxed);
        }
    });

    let t2 = spawn(move || {
        while !y.load(Ordering::Acquire) {}
        if x.load(Ordering::Acquire) {
            z.fetch_add(1, Ordering::Relaxed);
        }
    });
    t1.join().unwrap();
    t2.join().unwrap();

    let z = z.load(Ordering::SeqCst);
    // What are the possible value for z?
    //  - Is 0 possible?
    //    Restrictions
    //      We know that t1 must run "after" tx otherwise infinite loop
    //      We know that t2 must run "after" ty otherwise infinite loop
         // Given that 
         //   .. tx .. t1 ..
         //      ty t2 tx t1 -> t1 will incr z (because tx ran before it)
         //      ty tx t2 t1 .. t1 and t2 will incr z (because ty ran before t2 and tx ran before t1)
         //      .. tx .. t1 ty t2 -> t2 will incr z  (becasue y is true(loop will break) and x is also true so incr can happen)
    //  - Is 1 possible?
    //    Yes: tx, t1, ty, t2
    //  - Is 2 possible?
    //    Yes: tx, ty, t1, t2
}
// TODO: Learn MESI protocol
#[test]
fn too_relaxed() {
    fn mutex_test() {
        let l: &'static _ = Box::leak(Box::new(Mutex::new(0)));
        let handles: Vec<_> = (0..100)
            .map(|_| {
                spawn(move || {
                    for _ in 0..1000 {
                        l.with_lock(|v| {
                            *v += 1;
                        })
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(l.with_lock(|v| *v), 100 * 1000);
    }

    let x: &'static _ = Box::leak(Box::new(AtomicUsize::new(0)));
    let y: &'static _ = Box::leak(Box::new(AtomicUsize::new(0)));
    let t1 = spawn(move || {
        let r1 = y.load(Ordering::Relaxed);
        x.store(r1, Ordering::Relaxed);
        r1
    });

    let t2 = spawn(move || {
        let r2 = x.load(Ordering::Relaxed);
        y.store(42, Ordering::Relaxed);
        r2
    });

    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();
    // r1 == r2 == 42 :)
    // Why?

    // Modification Order
    // MO(x): 0 42
    // MO(y): 0 42

    // With Order Relaxed, when you load a value you can read any value
    // written by any thread, there is no restriction of when last a write happend relative to you
}
