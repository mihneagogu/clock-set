use clock_set;

#[cfg(test)]
mod tests {
    use crate::clock_set::*;
    use std::sync::Arc;
    use std::thread;
    #[test]
    fn empty_construction_doesnt_reformat_harddisk() {
        let set = CSet::<()>::new();
        assert_eq!(0, set.size());
    }

    #[test]
    fn inserting_values_makes_elements_unique() {
        let set = Arc::new(CSet::<u32>::new());
        for _ in 1..5 {
            insert_200(&set);
        }
        if let Ok(set) = Arc::try_unwrap(set) {
            assert_eq!(set.size(), 200);
        } else {
            unreachable!()
        }
    }

    fn insert_200(set: &Arc<CSet<u32>>) {
        let mut ts = vec![];
        for i in 1..=200u32 {
            let arc_set = Arc::clone(&set);
            let handle = thread::spawn(move || {
                &arc_set.insert_with_key(i, i as u64);
            });
            ts.push(handle);
        }
        for jh in ts.into_iter() {
            assert!(jh.join().is_ok());
        }
    }

    #[test]
    fn deleting_actually_deletes_values() {
        let set = Arc::new(CSet::<u32>::new());
        insert_200(&set);
        let mut ts = vec![];
        for round in 1..=5 {
            for i in 1..=200 {
                if i % 2 == 0 {
                    let s = Arc::clone(&set);
                    let handle = thread::spawn(move || {
                        let deleted = s.remove_with_key(i);
                        assert_eq!(deleted, round == 1);
                    });
                    ts.push(handle);
                }
            }
        }
        for jh in ts.into_iter() {
            assert!(jh.join().is_ok());
        }
        if let Ok(set) = Arc::try_unwrap(set) {
            assert_eq!(set.size(), 100);
        } else {
            unreachable!()
        }
    }

    #[test]
    /// This test is to be used in Miri. We just make a bunch of concurrent
    /// operations and ensure that nothing blows up
    fn multiple_operations_run_normally() {
        let set = Arc::new(CSet::<u32>::new());
        insert_200(&set);
        let mut ts = vec![];
        for round in 1..=5 {
            for i in 1..=200 {
                let s = Arc::clone(&set);
                match round {
                    1 | 3 => {
                        let handle = thread::spawn(move || {
                            s.insert_with_key(i, i as u64);
                        });
                        ts.push(handle);
                    }
                    2 | 4 => {
                        let handle = thread::spawn(move || {
                            s.remove_with_key(i as u64);
                        });
                        ts.push(handle);
                    }
                    _ => {
                        let handle = thread::spawn(move || {
                            s.contains_with_key(i as u64);
                        });
                        ts.push(handle);
                    }
                }
            }
        }
        for jh in ts.into_iter() {
            assert!(jh.join().is_ok());
        }
    }
}
