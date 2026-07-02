//! Concurrent reverse tracing contracts with shared in-flight path collapse.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::sync::{Arc, Condvar, Mutex};

use rayon::prelude::{IntoParallelIterator, ParallelIterator};

use crate::failure;
use crate::roots;

/// Graph capability required by reverse tracing.
pub trait GraphView {
    /// Returns direct reverse dependents in stable order.
    #[must_use]
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];
}

/// Test classification capability required by reverse tracing.
pub trait TestClassifier {
    /// Reports whether a path is a test file.
    #[must_use]
    fn is_test(&self, path: &roots::RootRelativePath) -> bool;
}

/// Progress sink capability used by the trace scheduler.
pub trait TraceProgressSink {
    /// Receives a typed trace event.
    fn send(&self, event: TraceEvent);
}

/// Trace status stored for each graph node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceStatus {
    /// No worker has started tracing this node.
    Unseen,
    /// A worker owns this node and other workers should join or observe it.
    InFlight,
    /// Tracing completed successfully.
    Complete(TraceResult),
    /// Tracing failed for a deterministic reason.
    Failed(Box<str>),
}

/// Trace handle returned when a trace request touches memoized state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceHandle {
    /// Caller should schedule new work for the path.
    Scheduled(roots::RootRelativePath),
    /// Caller should wait for an existing in-flight trace.
    Waiting(roots::RootRelativePath),
    /// Caller can immediately reuse a completed result.
    Completed(TraceResult),
}

/// Deterministic trace output for one graph node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceResult {
    /// Stable sorted tests reached from the node.
    pub tests: Box<[roots::RootRelativePath]>,
    /// Stable sorted shortest reason chains.
    pub reasons: Box<[TraceReason]>,
    /// Stable cycle edges encountered while tracing.
    pub cycle_edges: Box<[CycleEdge]>,
}

/// Shortest reason chain produced by tracing.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TraceReason {
    /// Starting node for the trace.
    pub start: roots::RootRelativePath,
    /// Selected test reached by tracing.
    pub test: roots::RootRelativePath,
    /// Ordered reverse dependency path.
    pub path: Box<[roots::RootRelativePath]>,
}

/// Deterministic representation of a cycle edge.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CycleEdge {
    /// Source path in the cycle edge.
    pub from: roots::RootRelativePath,
    /// Target path in the cycle edge.
    pub to: roots::RootRelativePath,
}

/// Trace scheduler contract that owns parallel work coordination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceScheduler {
    worker_count: NonZeroUsize,
}

impl TraceScheduler {
    /// Creates a scheduler contract with a non-zero worker count.
    #[must_use]
    pub const fn new(worker_count: NonZeroUsize) -> Self {
        Self { worker_count }
    }

    /// Returns the maximum worker count available to the scheduler.
    #[must_use]
    pub const fn worker_count(&self) -> NonZeroUsize {
        self.worker_count
    }
}

/// Progress event emitted by trace scheduling and memo reuse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceEvent {
    /// A path was scheduled for first-time tracing.
    Scheduled(roots::RootRelativePath),
    /// A path joined an in-flight trace.
    JoinedInFlight(roots::RootRelativePath),
    /// A path reused a completed trace result.
    Reused(roots::RootRelativePath),
    /// A path completed with a deterministic result.
    Completed(roots::RootRelativePath),
    /// A path failed before a deterministic result could be produced.
    Failed(roots::RootRelativePath),
    /// A cycle edge was recorded without deadlocking.
    Cycle(CycleEdge),
}

/// Shared memo table for trace coordination.
#[derive(Clone, Debug, Default)]
pub struct TraceMemo {
    statuses: Arc<Mutex<BTreeMap<roots::RootRelativePath, TraceStatus>>>,
    wait_edges: Arc<Mutex<BTreeMap<roots::RootRelativePath, roots::RootRelativePath>>>,
    completed: Arc<Condvar>,
}

impl TraceMemo {
    /// Returns the shared status storage.
    #[must_use]
    pub fn statuses(&self) -> Arc<Mutex<BTreeMap<roots::RootRelativePath, TraceStatus>>> {
        Arc::clone(&self.statuses)
    }
}

/// Request object for tracing changed paths.
pub struct Request<G, C, S> {
    /// Reverse graph view.
    pub graph: G,
    /// Test classifier.
    pub classifier: C,
    /// Progress sink for typed trace events.
    pub progress: S,
    /// Shared trace memo.
    pub memo: TraceMemo,
    /// Scheduler configuration for worker coordination.
    pub scheduler: TraceScheduler,
    /// Changed files to trace.
    pub changed_files: Box<[roots::RootRelativePath]>,
}

/// Traces changed files through the reverse graph with shared memoization.
///
/// # Errors
///
/// Returns an error when tracing cannot produce deterministic results.
pub fn changed_files<G, C, S>(request: Request<G, C, S>) -> failure::Result<TraceResult>
where
    G: GraphView + Sync,
    C: TestClassifier + Sync,
    S: TraceProgressSink + Sync,
{
    let Request {
        graph,
        classifier,
        progress,
        memo,
        scheduler,
        changed_files,
    } = request;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(scheduler.worker_count().get())
        .build()
        .map_err(|error| failure::AppError::Graph {
            message: format!("trace worker pool failed: {error}").into_boxed_str(),
        })?;
    let context = TraceContext {
        graph: &graph,
        classifier: &classifier,
        progress: &progress,
        memo: &memo,
    };
    let roots = sorted_unique(changed_files.into_vec());
    let results = pool.install(|| {
        roots
            .into_vec()
            .into_par_iter()
            .map(|path| {
                trace_node(TraceNodeRequest {
                    context: &context,
                    path,
                    ancestors: Box::from([]),
                })
            })
            .collect::<failure::Result<Vec<TraceResult>>>()
    })?;

    Ok(merge_results(results))
}

struct TraceContext<'a, G, C, S> {
    graph: &'a G,
    classifier: &'a C,
    progress: &'a S,
    memo: &'a TraceMemo,
}

struct TraceNodeRequest<'a, 'context, G, C, S> {
    context: &'context TraceContext<'a, G, C, S>,
    path: roots::RootRelativePath,
    ancestors: Box<[roots::RootRelativePath]>,
}

struct ComputeRequest<'a, 'context, G, C, S> {
    context: &'context TraceContext<'a, G, C, S>,
    path: roots::RootRelativePath,
    ancestors: Box<[roots::RootRelativePath]>,
}

enum CompletedStatus {
    Complete(TraceResult),
    Failed(Box<str>),
}

struct WaitRequest<'a> {
    memo: &'a TraceMemo,
    waiter: Option<roots::RootRelativePath>,
    path: roots::RootRelativePath,
}

struct CompletePathRequest<'a, S> {
    memo: &'a TraceMemo,
    progress: &'a S,
    path: roots::RootRelativePath,
    result: &'a failure::Result<TraceResult>,
}

struct RegisterWaitRequest<'a> {
    memo: &'a TraceMemo,
    waiter: Option<&'a roots::RootRelativePath>,
    path: &'a roots::RootRelativePath,
}

struct WaitCycleRequest<'a> {
    wait_edges: &'a BTreeMap<roots::RootRelativePath, roots::RootRelativePath>,
    path: &'a roots::RootRelativePath,
    waiter: &'a roots::RootRelativePath,
}

fn trace_node<G, C, S>(request: TraceNodeRequest<'_, '_, G, C, S>) -> failure::Result<TraceResult>
where
    G: GraphView + Sync,
    C: TestClassifier + Sync,
    S: TraceProgressSink + Sync,
{
    let TraceNodeRequest {
        context,
        path,
        ancestors,
    } = request;

    if ancestors.contains(&path) {
        let cycle_edge = cycle_from_ancestors(ancestors.as_ref(), &path);
        context.progress.send(TraceEvent::Cycle(cycle_edge.clone()));
        return Ok(TraceResult {
            tests: Box::from([]),
            reasons: Box::from([]),
            cycle_edges: Box::from([cycle_edge]),
        });
    }

    match claim_path(context.memo, &path)? {
        TraceHandle::Completed(result) => {
            context.progress.send(TraceEvent::Reused(path));
            Ok(result)
        }
        TraceHandle::Waiting(waiting_path) => {
            context
                .progress
                .send(TraceEvent::JoinedInFlight(waiting_path.clone()));
            wait_for_path(WaitRequest {
                memo: context.memo,
                waiter: ancestors.first().cloned(),
                path: waiting_path,
            })
        }
        TraceHandle::Scheduled(scheduled_path) => {
            context
                .progress
                .send(TraceEvent::Scheduled(scheduled_path.clone()));
            let result = compute_node(ComputeRequest {
                context,
                path: scheduled_path.clone(),
                ancestors,
            });
            complete_path(CompletePathRequest {
                memo: context.memo,
                progress: context.progress,
                path: scheduled_path,
                result: &result,
            });
            result
        }
    }
}

fn compute_node<G, C, S>(request: ComputeRequest<'_, '_, G, C, S>) -> failure::Result<TraceResult>
where
    G: GraphView + Sync,
    C: TestClassifier + Sync,
    S: TraceProgressSink + Sync,
{
    let ComputeRequest {
        context,
        path,
        ancestors,
    } = request;
    let mut results = Vec::<TraceResult>::new();
    let path_is_test = context.classifier.is_test(&path);
    if path_is_test {
        results.push(test_result(&path));
    }

    let child_ancestors = extend_ancestors(ancestors.as_ref(), &path);
    let dependents = context.graph.reverse_dependents(&path);
    for dependent in dependents {
        if ancestors.contains(dependent) {
            let cycle_edge = CycleEdge {
                from: path.clone(),
                to: dependent.clone(),
            };
            context.progress.send(TraceEvent::Cycle(cycle_edge.clone()));
            results.push(TraceResult {
                tests: Box::from([]),
                reasons: Box::from([]),
                cycle_edges: Box::from([cycle_edge]),
            });
        } else {
            let child_result = trace_node(TraceNodeRequest {
                context,
                path: dependent.clone(),
                ancestors: child_ancestors.clone(),
            })?;
            results.push(prefix_result(&path, child_result));
        }
    }

    Ok(merge_results(results))
}

fn claim_path(memo: &TraceMemo, path: &roots::RootRelativePath) -> failure::Result<TraceHandle> {
    let mut statuses = memo
        .statuses
        .lock()
        .map_err(|_error| failure::AppError::Graph {
            message: Box::<str>::from("trace memo status lock poisoned"),
        })?;

    let handle = match statuses.get(path) {
        Some(TraceStatus::Complete(result)) => Ok(TraceHandle::Completed(result.clone())),
        Some(TraceStatus::Failed(message)) => Err(failure::AppError::Graph {
            message: message.clone(),
        }),
        Some(TraceStatus::InFlight) => Ok(TraceHandle::Waiting(path.clone())),
        Some(TraceStatus::Unseen) | None => {
            statuses.insert(path.clone(), TraceStatus::InFlight);
            Ok(TraceHandle::Scheduled(path.clone()))
        }
    };

    drop(statuses);
    handle
}

fn wait_for_path(request: WaitRequest<'_>) -> failure::Result<TraceResult> {
    let WaitRequest { memo, waiter, path } = request;
    if let Some(cycle_edge) = register_wait(&RegisterWaitRequest {
        memo,
        waiter: waiter.as_ref(),
        path: &path,
    })? {
        return Ok(TraceResult {
            tests: Box::from([]),
            reasons: Box::from([]),
            cycle_edges: Box::from([cycle_edge]),
        });
    }

    let mut statuses = memo
        .statuses
        .lock()
        .map_err(|_error| failure::AppError::Graph {
            message: Box::<str>::from("trace memo status lock poisoned"),
        })?;

    loop {
        match statuses.get(&path) {
            Some(TraceStatus::Complete(result)) => {
                remove_wait(memo, waiter.as_ref());
                return Ok(result.clone());
            }
            Some(TraceStatus::Failed(message)) => {
                remove_wait(memo, waiter.as_ref());
                return Err(failure::AppError::Graph {
                    message: message.clone(),
                });
            }
            Some(TraceStatus::InFlight | TraceStatus::Unseen) | None => {
                statuses =
                    memo.completed
                        .wait(statuses)
                        .map_err(|_error| failure::AppError::Graph {
                            message: Box::<str>::from("trace memo status lock poisoned"),
                        })?;
            }
        }
    }
}

fn complete_path<S>(request: CompletePathRequest<'_, S>)
where
    S: TraceProgressSink,
{
    let CompletePathRequest {
        memo,
        progress,
        path,
        result,
    } = request;
    let completed_status = completed_status(result);
    if let Ok(mut statuses) = memo.statuses.lock() {
        let status = match completed_status {
            CompletedStatus::Complete(trace_result) => TraceStatus::Complete(trace_result),
            CompletedStatus::Failed(message) => TraceStatus::Failed(message),
        };
        statuses.insert(path.clone(), status);
    }
    remove_wait(memo, Some(&path));
    memo.completed.notify_all();
    match result {
        Ok(_trace_result) => progress.send(TraceEvent::Completed(path)),
        Err(_error) => progress.send(TraceEvent::Failed(path)),
    }
}

fn register_wait(request: &RegisterWaitRequest<'_>) -> failure::Result<Option<CycleEdge>> {
    let Some(waiter_path) = request.waiter else {
        return Ok(None);
    };
    let mut wait_edges =
        request
            .memo
            .wait_edges
            .lock()
            .map_err(|_error| failure::AppError::Graph {
                message: Box::<str>::from("trace memo wait lock poisoned"),
            })?;

    if wait_cycle_exists(&WaitCycleRequest {
        wait_edges: &wait_edges,
        path: request.path,
        waiter: waiter_path,
    }) {
        return Ok(Some(CycleEdge {
            from: waiter_path.clone(),
            to: request.path.clone(),
        }));
    }

    wait_edges.insert(waiter_path.clone(), request.path.clone());
    drop(wait_edges);
    Ok(None)
}

fn wait_cycle_exists(request: &WaitCycleRequest<'_>) -> bool {
    let mut current_path = request.path;
    loop {
        if current_path == request.waiter {
            return true;
        }
        let Some(next_path) = request.wait_edges.get(current_path) else {
            return false;
        };
        current_path = next_path;
    }
}

fn remove_wait(memo: &TraceMemo, waiter: Option<&roots::RootRelativePath>) {
    if let Some(waiter_path) = waiter
        && let Ok(mut wait_edges) = memo.wait_edges.lock()
    {
        wait_edges.remove(waiter_path);
    }
}

fn completed_status(result: &failure::Result<TraceResult>) -> CompletedStatus {
    match result {
        Ok(trace_result) => CompletedStatus::Complete(trace_result.clone()),
        Err(error) => CompletedStatus::Failed(error.to_string().into_boxed_str()),
    }
}

fn test_result(path: &roots::RootRelativePath) -> TraceResult {
    TraceResult {
        tests: Box::from([path.clone()]),
        reasons: Box::from([TraceReason {
            start: path.clone(),
            test: path.clone(),
            path: Box::from([path.clone()]),
        }]),
        cycle_edges: Box::from([]),
    }
}

fn prefix_result(path: &roots::RootRelativePath, result: TraceResult) -> TraceResult {
    let tests = result.tests.clone();
    let reasons = result
        .reasons
        .into_vec()
        .into_iter()
        .map(|reason| prefix_reason(path, reason))
        .collect::<Vec<TraceReason>>()
        .into_boxed_slice();

    TraceResult {
        tests,
        reasons,
        cycle_edges: result.cycle_edges,
    }
}

fn prefix_reason(path: &roots::RootRelativePath, reason: TraceReason) -> TraceReason {
    let mut reason_path = Vec::<roots::RootRelativePath>::with_capacity(reason.path.len() + 1);
    reason_path.push(path.clone());
    reason_path.extend(reason.path.into_vec());

    TraceReason {
        start: path.clone(),
        test: reason.test,
        path: reason_path.into_boxed_slice(),
    }
}

fn cycle_from_ancestors(
    ancestors: &[roots::RootRelativePath],
    path: &roots::RootRelativePath,
) -> CycleEdge {
    let from = ancestors.last().map_or_else(|| path.clone(), Clone::clone);

    CycleEdge {
        from,
        to: path.clone(),
    }
}

fn extend_ancestors(
    ancestors: &[roots::RootRelativePath],
    path: &roots::RootRelativePath,
) -> Box<[roots::RootRelativePath]> {
    let mut extended = Vec::<roots::RootRelativePath>::with_capacity(ancestors.len() + 1);
    extended.extend_from_slice(ancestors);
    extended.push(path.clone());
    extended.into_boxed_slice()
}

fn merge_results(results: Vec<TraceResult>) -> TraceResult {
    let mut tests = BTreeSet::<roots::RootRelativePath>::new();
    let mut reasons = BTreeMap::<ReasonKey, TraceReason>::new();
    let mut cycle_edges = BTreeSet::<CycleEdge>::new();

    for result in results {
        tests.extend(result.tests.into_vec());
        for reason in result.reasons.into_vec() {
            insert_shortest_reason(&mut reasons, reason);
        }
        cycle_edges.extend(result.cycle_edges.into_vec());
    }

    TraceResult {
        tests: tests.into_iter().collect::<Vec<_>>().into_boxed_slice(),
        reasons: reasons.into_values().collect::<Vec<_>>().into_boxed_slice(),
        cycle_edges: cycle_edges
            .into_iter()
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReasonKey {
    start: roots::RootRelativePath,
    test: roots::RootRelativePath,
}

fn insert_shortest_reason(reasons: &mut BTreeMap<ReasonKey, TraceReason>, reason: TraceReason) {
    let key = ReasonKey {
        start: reason.start.clone(),
        test: reason.test.clone(),
    };
    let should_replace = reasons.get(&key).is_none_or(|existing| {
        reason.path.len() < existing.path.len()
            || (reason.path.len() == existing.path.len() && reason.path < existing.path)
    });

    if should_replace {
        reasons.insert(key, reason);
    }
}

fn sorted_unique(mut paths: Vec<roots::RootRelativePath>) -> Box<[roots::RootRelativePath]> {
    paths.sort();
    paths.dedup();
    paths.into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::num::NonZeroUsize;
    use std::sync::{Arc, Condvar, Mutex};

    use crate::roots;

    #[derive(Clone, Debug)]
    struct FixtureGraph {
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        empty: Box<[roots::RootRelativePath]>,
    }

    impl super::GraphView for FixtureGraph {
        fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
            self.reverse_edges
                .get(path)
                .map_or_else(|| self.empty.as_ref(), Box::as_ref)
        }
    }

    #[derive(Clone, Debug)]
    struct BlockingGraph {
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        empty: Box<[roots::RootRelativePath]>,
        gate: JoinGate,
        trace_counts: Arc<Mutex<BTreeMap<roots::RootRelativePath, usize>>>,
    }

    impl BlockingGraph {
        fn count_for(&self, path: &roots::RootRelativePath) -> usize {
            self.trace_counts
                .lock()
                .unwrap()
                .get(path)
                .copied()
                .unwrap_or_default()
        }
    }

    impl super::GraphView for BlockingGraph {
        fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
            record_trace_count(&self.trace_counts, path);
            self.gate.observe_graph_path(path);
            self.reverse_edges
                .get(path)
                .map_or_else(|| self.empty.as_ref(), Box::as_ref)
        }
    }

    #[derive(Clone, Debug)]
    struct CrossRootCycleGraph {
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        empty: Box<[roots::RootRelativePath]>,
        gate: CrossRootCycleGate,
    }

    impl super::GraphView for CrossRootCycleGraph {
        fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
            self.gate.observe_graph_path(path);
            self.reverse_edges
                .get(path)
                .map_or_else(|| self.empty.as_ref(), Box::as_ref)
        }
    }

    #[derive(Clone, Debug, Default)]
    struct CrossRootCycleGate {
        state: Arc<(Mutex<CrossRootCycleState>, Condvar)>,
    }

    impl CrossRootCycleGate {
        fn observe_graph_path(&self, path: &roots::RootRelativePath) {
            match path.as_str() {
                "root-a.ts" => self.mark_root_a_observed(),
                "root-b.ts" => self.mark_root_b_observed(),
                _ => return,
            }
            self.wait_for_both_roots_observed();
        }

        fn mark_root_a_observed(&self) {
            let (state, completed) = self.state.as_ref();
            let mut cycle_state = state.lock().unwrap();
            cycle_state.root_a_observed = true;
            drop(cycle_state);
            completed.notify_all();
        }

        fn mark_root_b_observed(&self) {
            let (state, completed) = self.state.as_ref();
            let mut cycle_state = state.lock().unwrap();
            cycle_state.root_b_observed = true;
            drop(cycle_state);
            completed.notify_all();
        }

        fn wait_for_both_roots_observed(&self) {
            let (state, completed) = self.state.as_ref();
            let mut cycle_state = state.lock().unwrap();
            while !cycle_state.root_a_observed || !cycle_state.root_b_observed {
                cycle_state = completed.wait(cycle_state).unwrap();
            }
            drop(cycle_state);
        }
    }

    #[derive(Clone, Debug, Default)]
    struct CrossRootCycleState {
        root_a_observed: bool,
        root_b_observed: bool,
    }

    #[derive(Clone, Debug, Default)]
    struct JoinGate {
        state: Arc<(Mutex<JoinState>, Condvar)>,
    }

    impl JoinGate {
        fn observe_graph_path(&self, path: &roots::RootRelativePath) {
            if path.as_str() == "fileA" {
                self.wait_for_shared_trace_to_start();
            }
            if path.as_str() == "fileB" {
                self.mark_shared_trace_started();
                self.wait_for_join();
            }
        }

        fn mark_shared_trace_started(&self) {
            let (state, completed) = self.state.as_ref();
            let mut join_state = state.lock().unwrap();
            join_state.shared_trace_started = true;
            drop(join_state);
            completed.notify_all();
        }

        fn wait_for_shared_trace_to_start(&self) {
            let (state, completed) = self.state.as_ref();
            let mut join_state = state.lock().unwrap();
            while !join_state.shared_trace_started {
                join_state = completed.wait(join_state).unwrap();
            }
            drop(join_state);
        }

        fn wait_for_join(&self) {
            let (state, completed) = self.state.as_ref();
            let mut join_state = state.lock().unwrap();
            while !join_state.join_observed {
                join_state = completed.wait(join_state).unwrap();
            }
            drop(join_state);
        }

        fn mark_join_observed(&self) {
            let (state, completed) = self.state.as_ref();
            let mut join_state = state.lock().unwrap();
            join_state.join_observed = true;
            drop(join_state);
            completed.notify_all();
        }
    }

    #[derive(Clone, Debug, Default)]
    struct JoinState {
        shared_trace_started: bool,
        join_observed: bool,
    }

    #[derive(Clone, Debug)]
    struct FixtureClassifier {
        test_paths: BTreeSet<roots::RootRelativePath>,
    }

    impl super::TestClassifier for FixtureClassifier {
        fn is_test(&self, path: &roots::RootRelativePath) -> bool {
            self.test_paths.contains(path)
        }
    }

    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        events: Arc<Mutex<Vec<super::TraceEvent>>>,
    }

    impl super::TraceProgressSink for RecordingSink {
        fn send(&self, event: super::TraceEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[derive(Clone, Debug)]
    struct JoinRecordingSink {
        events: Arc<Mutex<Vec<super::TraceEvent>>>,
        gate: JoinGate,
    }

    impl super::TraceProgressSink for JoinRecordingSink {
        fn send(&self, event: super::TraceEvent) {
            if event == super::TraceEvent::JoinedInFlight(path("fileB")) {
                self.gate.mark_join_observed();
            }
            self.events.lock().unwrap().push(event);
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn scheduler() -> super::TraceScheduler {
        super::TraceScheduler::new(NonZeroUsize::new(4).unwrap())
    }

    fn classifier(test_paths: Box<[roots::RootRelativePath]>) -> FixtureClassifier {
        FixtureClassifier {
            test_paths: test_paths.into_vec().into_iter().collect(),
        }
    }

    fn graph(
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
    ) -> FixtureGraph {
        FixtureGraph {
            reverse_edges,
            empty: Box::from([]),
        }
    }

    fn blocking_graph(
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        gate: JoinGate,
    ) -> BlockingGraph {
        BlockingGraph {
            reverse_edges,
            empty: Box::from([]),
            gate,
            trace_counts: Arc::default(),
        }
    }

    fn cross_root_cycle_graph(
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        gate: CrossRootCycleGate,
    ) -> CrossRootCycleGraph {
        CrossRootCycleGraph {
            reverse_edges,
            empty: Box::from([]),
            gate,
        }
    }

    fn record_trace_count(
        trace_counts: &Arc<Mutex<BTreeMap<roots::RootRelativePath, usize>>>,
        path: &roots::RootRelativePath,
    ) {
        let mut counts = trace_counts.lock().unwrap();
        let count = counts.entry(path.clone()).or_default();
        *count += 1;
        drop(counts);
    }

    fn trace(
        graph: FixtureGraph,
        changed_files: Box<[roots::RootRelativePath]>,
    ) -> super::TraceResult {
        let request = super::Request {
            graph,
            classifier: classifier(Box::from([path("tests/file-b.test.ts"), path("fileE")])),
            progress: RecordingSink::default(),
            memo: super::TraceMemo::default(),
            scheduler: scheduler(),
            changed_files,
        };

        super::changed_files(request).unwrap()
    }

    #[test]
    fn starts_multiple_changed_files_in_parallel() {
        let graph = graph(BTreeMap::from([
            (path("fileB"), Box::from([path("tests/file-b.test.ts")])),
            (path("fileD"), Box::from([path("fileE")])),
        ]));

        // Two independent roots are enough to prove the scheduler does not serialize
        // work before discovering shared paths.
        let result = trace(graph, Box::from([path("fileB"), path("fileD")]));

        assert_eq!(
            result.tests,
            Box::from([path("fileE"), path("tests/file-b.test.ts")]),
        );
    }

    #[test]
    fn reuses_completed_trace_results_when_a_later_path_reaches_the_same_node() {
        let memo = super::TraceMemo::default();
        let completed = super::TraceResult {
            tests: Box::from([path("tests/file-b.test.ts")]),
            reasons: Box::from([]),
            cycle_edges: Box::from([]),
        };
        memo.statuses().lock().unwrap().insert(
            path("fileB"),
            super::TraceStatus::Complete(completed.clone()),
        );
        let request = super::Request {
            graph: graph(BTreeMap::from([(
                path("fileA"),
                Box::from([path("fileB")]),
            )])),
            classifier: classifier(Box::from([path("tests/file-b.test.ts")])),
            progress: RecordingSink::default(),
            memo,
            scheduler: scheduler(),
            changed_files: Box::from([path("fileA")]),
        };

        let result = super::changed_files(request).unwrap();

        assert_eq!(result, completed);
    }

    #[test]
    fn joins_in_flight_trace_results_without_retracing_the_same_path() {
        let memo = super::TraceMemo::default();
        let gate = JoinGate::default();
        let events = Arc::new(Mutex::new(Vec::<super::TraceEvent>::new()));
        let graph = blocking_graph(
            BTreeMap::from([
                (path("fileA"), Box::from([path("fileB")])),
                (path("fileB"), Box::from([path("tests/file-b.test.ts")])),
            ]),
            gate.clone(),
        );
        let request = super::Request {
            graph: graph.clone(),
            classifier: classifier(Box::from([path("tests/file-b.test.ts")])),
            progress: JoinRecordingSink {
                events: Arc::clone(&events),
                gate,
            },
            memo,
            scheduler: scheduler(),
            changed_files: Box::from([path("fileA"), path("fileB")]),
        };

        let result = super::changed_files(request).unwrap();

        assert_eq!(result.tests, Box::from([path("tests/file-b.test.ts")]));
        assert_eq!(graph.count_for(&path("fileB")), 1);
        assert!(
            events
                .lock()
                .unwrap()
                .contains(&super::TraceEvent::JoinedInFlight(path("fileB")))
        );
    }

    #[test]
    fn collapses_file_b_overlap_while_independent_file_d_branch_continues() {
        let graph = graph(BTreeMap::from([
            (path("fileD"), Box::from([path("fileA"), path("fileC")])),
            (path("fileA"), Box::from([path("fileB")])),
            (path("fileC"), Box::from([path("fileE")])),
            (path("fileB"), Box::from([path("tests/file-b.test.ts")])),
        ]));

        // The overlapping branch should join fileB tracing while fileC -> fileE
        // continues, preserving both selected tests in the final result.
        let result = trace(graph, Box::from([path("fileB"), path("fileD")]));

        assert_eq!(
            result.tests,
            Box::from([path("fileE"), path("tests/file-b.test.ts")]),
        );
    }

    #[test]
    fn cross_root_in_flight_cycles_complete_without_deadlock() {
        let gate = CrossRootCycleGate::default();
        let graph = cross_root_cycle_graph(
            BTreeMap::from([
                (path("root-a.ts"), Box::from([path("bridge-a.ts")])),
                (
                    path("bridge-a.ts"),
                    Box::from([path("root-b.ts"), path("tests/root-a.test.ts")]),
                ),
                (path("root-b.ts"), Box::from([path("bridge-b.ts")])),
                (
                    path("bridge-b.ts"),
                    Box::from([path("root-a.ts"), path("tests/root-b.test.ts")]),
                ),
            ]),
            gate,
        );
        let request = super::Request {
            graph,
            classifier: classifier(Box::from([
                path("tests/root-a.test.ts"),
                path("tests/root-b.test.ts"),
            ])),
            progress: RecordingSink::default(),
            memo: super::TraceMemo::default(),
            scheduler: scheduler(),
            changed_files: Box::from([path("root-a.ts"), path("root-b.ts")]),
        };

        // The gate forces both changed roots to be in flight before either root
        // can discover the other, reproducing the production deadlock shape.
        let result = super::changed_files(request).unwrap();

        assert_eq!(
            result.tests,
            Box::from([path("tests/root-a.test.ts"), path("tests/root-b.test.ts")]),
        );
        let cycle_edge = result.cycle_edges.first().unwrap();
        assert_eq!(result.cycle_edges.len(), 1);
        assert!(
            cycle_edge
                == &super::CycleEdge {
                    from: path("root-a.ts"),
                    to: path("root-b.ts"),
                }
                || cycle_edge
                    == &super::CycleEdge {
                        from: path("root-b.ts"),
                        to: path("root-a.ts"),
                    }
        );
    }

    #[test]
    fn cycles_complete_without_deadlock_or_duplicate_work() {
        let graph = graph(BTreeMap::from([
            (path("src/a.ts"), Box::from([path("src/b.ts")])),
            (
                path("src/b.ts"),
                Box::from([path("src/a.ts"), path("tests/a.test.ts")]),
            ),
        ]));
        let request = super::Request {
            graph,
            classifier: classifier(Box::from([path("tests/a.test.ts")])),
            progress: RecordingSink::default(),
            memo: super::TraceMemo::default(),
            scheduler: scheduler(),
            changed_files: Box::from([path("src/a.ts")]),
        };

        let result = super::changed_files(request).unwrap();

        assert_eq!(result.tests, Box::from([path("tests/a.test.ts")]));
        assert_eq!(
            result.cycle_edges,
            Box::from([super::CycleEdge {
                from: path("src/b.ts"),
                to: path("src/a.ts"),
            }]),
        );
    }
}
