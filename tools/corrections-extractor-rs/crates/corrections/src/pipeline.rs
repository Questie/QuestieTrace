//! Parallel work, folded in order: worker threads take the items one at a time, and the calling
//! thread folds their results strictly in item order as they arrive.
//!
//! Workers stay at most `ahead` items past the fold, so one slow item never holds the others up
//! for long and finished results never pile up. A panic in the work stops every worker and the
//! run reports which item failed, instead of leaving the fold waiting for a result that never
//! comes.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Condvar, Mutex};

/// Shared between the fold and the workers: how far the fold got, and whether to stop.
struct Progress {
    folded: Mutex<usize>,
    changed: Condvar,
    stop: AtomicBool,
}

impl Progress {
    fn set_folded(&self, folded: usize) {
        *self.folded.lock().expect("no thread panics holding the lock") = folded;
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Taking the lock orders the flag before any waiter's next check.
        drop(self.folded.lock().expect("no thread panics holding the lock"));
        self.changed.notify_all();
    }

    /// Blocks until `index` is less than `ahead` items past the fold; false when stopping.
    fn wait_for_room(&self, index: usize, ahead: usize) -> bool {
        let mut folded = self.folded.lock().expect("no thread panics holding the lock");
        while index >= *folded + ahead && !self.stop.load(Ordering::SeqCst) {
            folded = self.changed.wait(folded).expect("no thread panics holding the lock");
        }
        !self.stop.load(Ordering::SeqCst)
    }
}

/// Stops the workers however the fold ends, a panic included, so the scope can join them.
struct StopOnDrop<'p>(&'p Progress);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Runs `work` over `items` on `threads` workers (each with its own `init` state) and hands each
/// result to `fold` on this thread, in the order of `items`. Err holds the index of the item
/// whose work panicked; the items before it were folded, none after it.
pub fn process_in_order<I, S, R>(
    items: &[I],
    threads: usize,
    ahead: usize,
    init: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, &I) -> R + Sync,
    mut fold: impl FnMut(usize, R),
) -> Result<(), usize>
where
    I: Sync,
    R: Send,
{
    let ahead = ahead.max(1);
    let next = AtomicUsize::new(0);
    let progress = Progress {
        folded: Mutex::new(0),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
    };
    // None: the work on that item panicked.
    let (results, received) = mpsc::channel::<(usize, Option<R>)>();
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            let results = results.clone();
            let (next, progress, init, work) = (&next, &progress, &init, &work);
            scope.spawn(move || {
                let mut state = init();
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    if index >= items.len() || !progress.wait_for_room(index, ahead) {
                        return;
                    }
                    let result = catch_unwind(AssertUnwindSafe(|| work(&mut state, &items[index]))).ok();
                    let failed = result.is_none();
                    // The fold may have stopped already: then nobody listens.
                    if results.send((index, result)).is_err() || failed {
                        return;
                    }
                }
            });
        }
        drop(results);

        let _stop_workers = StopOnDrop(&progress);
        let mut pending: HashMap<usize, Option<R>> = HashMap::new();
        let mut expected = 0;
        for (index, result) in received {
            pending.insert(index, result);
            while let Some(result) = pending.remove(&expected) {
                let Some(result) = result else { return Err(expected) };
                fold(expected, result);
                expected += 1;
                progress.set_folded(expected);
            }
            if expected == items.len() {
                break;
            }
        }
        assert_eq!(expected, items.len(), "every worker stopped before all items were done");
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn folds_results_in_item_order_whatever_order_the_workers_finish_in() {
        let items: Vec<u64> = (0..200).collect();
        let mut folded = Vec::new();
        // Later items finish first in each batch of 7.
        let work = |_: &mut (), &item: &u64| {
            std::thread::sleep(Duration::from_micros((7 - item % 7) * 200));
            item * 10
        };
        process_in_order(&items, 4, 16, || (), work, |index, result| folded.push((index, result))).unwrap();
        let expected: Vec<(usize, u64)> = items.iter().map(|&i| (i as usize, i * 10)).collect();
        assert_eq!(folded, expected);
    }

    #[test]
    fn workers_never_run_more_than_ahead_items_past_the_fold() {
        let items: Vec<usize> = (0..300).collect();
        let folded = AtomicUsize::new(0);
        let furthest = AtomicUsize::new(0);
        let work = |_: &mut (), &item: &usize| {
            let lead = item - folded.load(Ordering::SeqCst);
            furthest.fetch_max(lead, Ordering::SeqCst);
        };
        process_in_order(
            &items,
            8,
            5,
            || (),
            work,
            |index, ()| {
                // A slow fold makes the workers wait.
                std::thread::sleep(Duration::from_micros(50));
                folded.store(index + 1, Ordering::SeqCst);
            },
        )
        .unwrap();
        assert!(
            furthest.load(Ordering::SeqCst) < 5,
            "a worker ran {} items ahead",
            furthest.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn a_panicking_item_stops_the_run_and_names_the_item_instead_of_hanging() {
        let items: Vec<usize> = (0..500).collect();
        let mut folded = Vec::new();
        let result = process_in_order(
            &items,
            4,
            8,
            || (),
            |_, &item| {
                assert_ne!(item, 37, "the work fails on item 37");
                item
            },
            |index, _| folded.push(index),
        );
        assert_eq!(result, Err(37));
        assert_eq!(folded, (0..37).collect::<Vec<_>>(), "everything before the failure is folded, nothing after it");
    }

    #[test]
    fn each_worker_keeps_its_own_state() {
        let items: Vec<usize> = (0..100).collect();
        let mut total = 0;
        process_in_order(
            &items,
            3,
            6,
            || 0usize,
            |seen: &mut usize, _| {
                *seen += 1;
                *seen
            },
            |_, seen| total += usize::from(seen == 1),
        )
        .unwrap();
        assert!((1..=3).contains(&total), "one first item per worker that got work, got {total}");
    }

    #[test]
    fn no_items_and_one_thread_work() {
        let mut calls = 0;
        process_in_order(&[] as &[u8], 4, 4, || (), |_, _| (), |_, ()| calls += 1).unwrap();
        process_in_order(&[1u8, 2, 3], 1, 1, || (), |_, &i| i, |_, i| calls += i as usize).unwrap();
        assert_eq!(calls, 6);
    }
}
