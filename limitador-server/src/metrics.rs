use std::collections::HashMap;
use std::ops;
use std::time::{Duration, Instant};
use tracing::span::{Attributes, Id};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timings {
    idle: u64,
    busy: u64,
    last: Instant,
}

impl Timings {
    fn new() -> Self {
        Self {
            idle: 0,
            busy: 0,
            last: Instant::now(),
        }
    }
}

impl ops::Add for Timings {
    type Output = Self;

    fn add(self, _rhs: Self) -> Self {
        Self {
            busy: self.busy + _rhs.busy,
            idle: self.idle + _rhs.idle,
            last: if self.last < _rhs.last {
                _rhs.last
            } else {
                self.last
            },
        }
    }
}

impl ops::AddAssign for Timings {
    fn add_assign(&mut self, _rhs: Self) {
        *self = *self + _rhs
    }
}

impl From<Timings> for Duration {
    fn from(timings: Timings) -> Self {
        Duration::from_nanos(timings.idle + timings.busy)
    }
}

#[derive(Debug, Clone)]
struct SpanState {
    group_times: HashMap<String, Timings>,
}

impl SpanState {
    fn new(group: String) -> Self {
        Self {
            group_times: HashMap::from([(group, Timings::new())]),
        }
    }

    fn increment(&mut self, group: &String, timings: Timings) -> &mut Self {
        self.group_times
            .entry(group.to_string())
            .and_modify(|x| *x += timings)
            .or_insert(timings);
        self
    }
}

pub struct MetricsGroup<F: Fn(Timings)> {
    consumer: F,
    records: Vec<String>,
}

impl<F: Fn(Timings)> MetricsGroup<F> {
    pub fn new(consumer: F, records: Vec<String>) -> Self {
        Self { consumer, records }
    }
}

pub struct MetricsLayer<F: Fn(Timings)> {
    groups: HashMap<String, MetricsGroup<F>>,
}

impl<F: Fn(Timings)> MetricsLayer<F> {
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
        }
    }

    pub fn gather(mut self, aggregate: &str, consumer: F, records: Vec<&str>) -> Self {
        // TODO(adam-cattermole): does not handle case where aggregate already exists
        let rec = records.iter().map(|r| r.to_string()).collect();
        self.groups
            .entry(aggregate.to_string())
            .or_insert(MetricsGroup::new(consumer, rec));
        self
    }
}

impl<S, F: Fn(Timings) + 'static> Layer<S> for MetricsLayer<F>
where
    S: Subscriber,
    S: for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();
        let name = span.name();

        // if there's a parent
        if let Some(parent) = span.parent() {
            // if the parent has SpanState propagate to this span
            if let Some(parent_state) = parent.extensions_mut().get_mut::<SpanState>() {
                extensions.insert(parent_state.clone());
            }
        }

        // if we are an aggregator
        if self.groups.contains_key(name) {
            if let Some(span_state) = extensions.get_mut::<SpanState>() {
                // if the SpanState has come from parent and we must append
                // (we are a second level aggregator)
                span_state
                    .group_times
                    .entry(name.to_string())
                    .or_insert(Timings::new());
            } else {
                // otherwise create a new SpanState with ourselves
                extensions.insert(SpanState::new(name.to_string()))
            }
        }

        if let Some(span_state) = extensions.get_mut::<SpanState>() {
            // either we are an aggregator or nested within one
            for group in span_state.group_times.keys() {
                for record in &self.groups.get(group).unwrap().records {
                    if name == record {
                        extensions.insert(Timings::new());
                        return;
                    }
                }
            }
            // if here we are an intermediate span that should not be recorded
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();

        if let Some(timings) = extensions.get_mut::<Timings>() {
            let now = Instant::now();
            timings.idle += (now - timings.last).as_nanos() as u64;
            timings.last = now;
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();

        if let Some(timings) = extensions.get_mut::<Timings>() {
            let now = Instant::now();
            timings.busy += (now - timings.last).as_nanos() as u64;
            timings.last = now;
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let span = ctx.span(&id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();
        let name = span.name();
        let mut t: Option<Timings> = None;

        if let Some(timing) = extensions.get_mut::<Timings>() {
            let mut time = *timing;
            time.idle += (Instant::now() - time.last).as_nanos() as u64;
            t = Some(time);
        }

        if let Some(span_state) = extensions.get_mut::<SpanState>() {
            if let Some(timing) = t {
                let group_times = span_state.group_times.clone();
                // iterate over the groups this span belongs to
                'aggregate: for group in group_times.keys() {
                    // find the set of records related to these groups in the layer
                    for record in &self.groups.get(group).unwrap().records {
                        // if we are a record for this group then increment the relevant
                        // span-local timing and continue to the next group
                        if name == record {
                            span_state.increment(group, timing);
                            continue 'aggregate;
                        }
                    }
                }
            }
            // we have updated local span_state
            // but we need to bubble back up through parents
            // NOTE: this propagates the timings, ready to be cloned by next new span!
            if let Some(parent) = span.parent() {
                parent.extensions_mut().replace(span_state.clone());
            }
            // IF we are aggregator call consume function
            if let Some(metrics_group) = self.groups.get(name) {
                (metrics_group.consumer)(*span_state.group_times.get(name).unwrap())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MetricsLayer, SpanState, Timings};
    use std::time::Instant;

    #[test]
    fn timings_add() {
        let now = Instant::now();
        let t1 = Timings {
            idle: 5,
            busy: 5,
            last: now,
        };
        let t2 = Timings {
            idle: 3,
            busy: 5,
            last: now,
        };
        let t3 = t1 + t2;
        assert_eq!(
            t3,
            Timings {
                idle: 8,
                busy: 10,
                last: now
            }
        )
    }

    #[test]
    fn timings_add_assign() {
        let now = Instant::now();
        let mut t1 = Timings {
            idle: 5,
            busy: 5,
            last: now,
        };
        let t2 = Timings {
            idle: 3,
            busy: 5,
            last: now,
        };
        t1 += t2;
        assert_eq!(
            t1,
            Timings {
                idle: 8,
                busy: 10,
                last: now
            }
        )
    }

    #[test]
    fn span_state_increment() {
        let group = String::from("group");
        let mut span_state = SpanState::new(group.clone());
        let t1 = Timings {
            idle: 5,
            busy: 5,
            last: Instant::now(),
        };
        span_state.increment(&group, t1);
        assert_eq!(span_state.group_times.get(&group).unwrap().idle, t1.idle);
        assert_eq!(span_state.group_times.get(&group).unwrap().busy, t1.busy);
    }

    #[test]
    fn metrics_layer() {
        let consumer = |_| println!("group/record");
        let ml = MetricsLayer::new().gather("group", consumer, vec!["record"]);
        assert_eq!(ml.groups.get("group").unwrap().records, vec!["record"]);
    }
}
