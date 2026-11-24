//! Priority queue primitives shared by server chunk work and render jobs.
//! They know only a job's owning column and ordering metadata, not how a job runs.
use horizon_sim_data::fast_hash::FxHashSet;
use std::collections::BinaryHeap;
use std::sync::Mutex;

/// A queued item ranked by distance to its column and a stable insertion order.
pub struct Queued<J> {
    pub priority: i64,
    /// The column the job is for; `None` jobs (save/free) are always urgent.
    pub at: Option<(i32, i32)>,
    /// Added to the distance-based priority when the centre moves.
    pub bias: i64,
    pub order: u64,
    pub job: J,
}

impl<J> PartialEq for Queued<J> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.order == other.order
    }
}
impl<J> Eq for Queued<J> {}
impl<J> PartialOrd for Queued<J> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<J> Ord for Queued<J> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap: lower priority value = more urgent.
        other
            .priority
            .cmp(&self.priority)
            .then(other.order.cmp(&self.order))
    }
}

/// Recompute queued column priorities when the center or ticket ranges change.
pub fn rank_again<J>(queue: &mut BinaryHeap<Queued<J>>, distance: impl Fn(i32, i32) -> i64) {
    if queue.is_empty() {
        return;
    }
    let mut jobs = std::mem::take(queue).into_vec();
    for job in &mut jobs {
        if let Some((x, z)) = job.at {
            job.priority = distance(x, z) * 4 + job.bias;
        }
    }
    *queue = BinaryHeap::from(jobs);
}

/// Drop queued work for unloaded columns, retaining jobs without a column.
pub fn release_columns<J>(
    queue: &mut BinaryHeap<Queued<J>>,
    gone: &FxHashSet<(i32, i32)>,
) -> usize {
    if queue.is_empty() || gone.is_empty() {
        return 0;
    }
    let queued = std::mem::take(queue).into_vec();
    let before = queued.len();
    let kept: Vec<Queued<J>> = queued
        .into_iter()
        .filter(|job| !job.at.is_some_and(|at| gone.contains(&at)))
        .collect();
    let released = before - kept.len();
    *queue = BinaryHeap::from(kept);
    released
}

/// Synchronized priority queue; it stores opaque jobs and knows no renderer types.
pub struct JobQueue<J> {
    queue: Mutex<BinaryHeap<Queued<J>>>,
}

impl<J> Default for JobQueue<J> {
    fn default() -> Self {
        Self {
            queue: Mutex::new(BinaryHeap::new()),
        }
    }
}

impl<J> JobQueue<J> {
    pub fn push(&self, item: Queued<J>) {
        self.queue.lock().unwrap().push(item);
    }

    pub fn pop(&self) -> Option<Queued<J>> {
        self.queue.lock().unwrap().pop()
    }

    pub fn len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    pub fn capacity(&self) -> usize {
        self.queue.lock().unwrap().capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }

    pub fn peek_rank(&self) -> Option<(Option<(i32, i32)>, i64)> {
        self.queue
            .lock()
            .unwrap()
            .peek()
            .map(|item| (item.at, item.priority))
    }

    pub fn first_matching_rank(
        &self,
        mut matches: impl FnMut(&J) -> bool,
    ) -> Option<(Option<(i32, i32)>, i64)> {
        self.queue
            .lock()
            .unwrap()
            .iter()
            .find(|item| matches(&item.job))
            .map(|item| (item.at, item.priority))
    }

    pub fn reprioritize(&self, distance: impl Fn(i32, i32) -> i64) {
        rank_again(&mut self.queue.lock().unwrap(), distance);
    }

    /// Best-effort queue maintenance for unload cleanup.
    pub fn try_release_columns(&self, gone: &FxHashSet<(i32, i32)>) -> Option<usize> {
        self.queue
            .lock()
            .ok()
            .map(|mut queue| release_columns(&mut queue, gone))
    }

    /// Best-effort filtering for world-exit abort cleanup.
    pub fn try_retain(&self, mut keep: impl FnMut(&Queued<J>) -> bool) -> bool {
        let Ok(mut queue) = self.queue.lock() else {
            return false;
        };
        let retained: Vec<_> = std::mem::take(&mut *queue)
            .into_vec()
            .into_iter()
            .filter(|item| keep(item))
            .collect();
        *queue = BinaryHeap::from(retained);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{JobQueue, Queued, rank_again, release_columns};
    use horizon_sim_data::fast_hash::FxHashSet;
    use std::collections::BinaryHeap;

    fn queued(at: Option<(i32, i32)>, priority: i64, bias: i64, order: u64) -> Queued<()> {
        Queued {
            priority,
            at,
            bias,
            order,
            job: (),
        }
    }

    #[test]
    fn heap_is_nearest_first_and_stable_for_equal_priorities() {
        let queue = BinaryHeap::from(vec![
            queued(Some((4, 0)), 8, 0, 1),
            queued(Some((2, 0)), 4, 0, 2),
            queued(Some((3, 0)), 4, 0, 3),
        ]);
        let mut queue = queue;
        let popped: Vec<_> = std::iter::from_fn(|| queue.pop())
            .map(|q| q.order)
            .collect();
        assert_eq!(popped, [2, 3, 1]);
    }

    #[test]
    fn reprioritization_preserves_bias_and_non_column_jobs() {
        let mut queue = BinaryHeap::from(vec![
            queued(Some((1, 0)), 40, 3, 1),
            queued(Some((3, 0)), 4, 0, 2),
            queued(None, i64::MIN, 0, 3),
        ]);
        rank_again(&mut queue, |x, _| i64::from((x - 3).abs()));
        let items = queue.into_vec();
        assert!(items.iter().any(|q| q.order == 1 && q.priority == 11));
        assert!(items.iter().any(|q| q.order == 2 && q.priority == 0));
        assert!(items.iter().any(|q| q.order == 3 && q.priority == i64::MIN));
    }

    #[test]
    fn release_drops_only_jobs_for_unloaded_columns() {
        let mut queue = BinaryHeap::from(vec![
            queued(Some((1, 2)), 0, 0, 1),
            queued(Some((3, 4)), 4, 0, 2),
            queued(None, i64::MIN, 0, 3),
        ]);
        let gone: FxHashSet<_> = [(1, 2)].into_iter().collect();
        assert_eq!(release_columns(&mut queue, &gone), 1);
        assert_eq!(queue.len(), 2);
        assert!(queue.iter().any(|q| q.order == 2));
        assert!(queue.iter().any(|q| q.order == 3));
    }

    #[test]
    fn synchronized_queue_exposes_rank_and_pops_nearest_first() {
        let queue = JobQueue::default();
        queue.push(queued(Some((5, 0)), 20, 0, 1));
        queue.push(queued(Some((1, 0)), 4, 0, 2));
        assert_eq!(queue.peek_rank(), Some((Some((1, 0)), 4)));
        assert_eq!(queue.first_matching_rank(|_| true), Some((Some((1, 0)), 4)));
        assert_eq!(queue.pop().map(|item| item.order), Some(2));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn synchronized_queue_releases_columns_and_retains_saves() {
        let queue = JobQueue::default();
        queue.push(queued(Some((1, 2)), 0, 0, 1));
        queue.push(queued(Some((3, 4)), 4, 0, 2));
        queue.push(queued(None, i64::MIN, 0, 3));
        let gone: FxHashSet<_> = [(1, 2)].into_iter().collect();
        assert_eq!(queue.try_release_columns(&gone), Some(1));
        assert!(queue.try_retain(|item| item.at.is_none()));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop().map(|item| item.order), Some(3));
        assert!(queue.is_empty());
    }
}
