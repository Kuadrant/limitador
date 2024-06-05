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
    updated: bool,
}

impl Timings {
    fn new() -> Self {
        Self {
            idle: 0,
            busy: 0,
            last: Instant::now(),
            updated: false,
        }
    }
}

impl ops::Add for Timings {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            busy: self.busy + rhs.busy,
            idle: self.idle + rhs.idle,
            last: self.last.max(rhs.last),
            updated: self.updated || rhs.updated,
        }
    }
}

impl ops::AddAssign for Timings {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs
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
    fn new(group: &String) -> Self {
        let mut group_times = HashMap::new();
        group_times.insert(group.to_owned(), Timings::new());
        Self { group_times }
    }

    fn increment(&mut self, group: &str, timings: Timings) {
        self.group_times
            .entry(group.to_string())
            .and_modify(|x| *x += timings)
            .or_insert(timings);
    }
}

pub struct MetricsGroup {
    consumer: Box<fn(Timings)>,
    records: Vec<String>,
}

impl MetricsGroup {
    pub fn new(consumer: Box<fn(Timings)>, records: Vec<String>) -> Self {
        Self { consumer, records }
    }
}

#[derive(Default)]
pub struct MetricsLayer {
    groups: HashMap<String, MetricsGroup>,
}

impl MetricsLayer {
    pub fn gather(mut self, aggregate: &str, consumer: fn(Timings), records: Vec<&str>) -> Self {
        // TODO(adam-cattermole): does not handle case where aggregate already exists
        let rec = records.iter().map(|&r| r.to_string()).collect();
        self.groups
            .entry(aggregate.to_string())
            .or_insert_with(|| MetricsGroup::new(Box::new(consumer), rec));
        self
    }
}

impl<S> Layer<S> for MetricsLayer
where
    S: Subscriber,
    S: for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();
        let name = span.name().to_string();

        // if there's a parent
        if let Some(parent) = span.parent() {
            // if the parent has SpanState propagate to this span
            if let Some(parent_state) = parent.extensions_mut().get_mut::<SpanState>() {
                extensions.insert(parent_state.clone());
            }
        }

        // if we are an aggregator
        if self.groups.contains_key(&name) {
            if let Some(span_state) = extensions.get_mut::<SpanState>() {
                // if the SpanState has come from parent and we must append
                // (we are a second level aggregator)
                span_state
                    .group_times
                    .entry(name.clone())
                    .or_insert_with(Timings::new);
            } else {
                // otherwise create a new SpanState with ourselves
                extensions.insert(SpanState::new(&name))
            }
        }

        if let Some(span_state) = extensions.get_mut::<SpanState>() {
            // either we are an aggregator or nested within one
            for group in span_state.group_times.keys() {
                if self
                    .groups
                    .get(group)
                    .expect("Span state contains group times for an unconfigured group")
                    .records
                    .contains(&name)
                {
                    extensions.insert(Timings::new());
                    return;
                }
            }
            // if here we are an intermediate span that should not be recorded
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if let Some(timings) = span.extensions_mut().get_mut::<Timings>() {
                let now = Instant::now();
                timings.idle += (now - timings.last).as_nanos() as u64;
                timings.last = now;

                timings.updated = true;
            }
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if let Some(timings) = span.extensions_mut().get_mut::<Timings>() {
                let now = Instant::now();
                timings.busy += (now - timings.last).as_nanos() as u64;
                timings.last = now;
                timings.updated = true;
            }
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let span = ctx.span(&id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();
        let name = span.name().to_string();

        let timing = extensions.get_mut::<Timings>().map(|t| {
            let now = Instant::now();
            t.idle += (now - t.last).as_nanos() as u64;
            *t
        });

        if let Some(span_state) = extensions.get_mut::<SpanState>() {
            if let Some(timing) = timing {
                // iterate over the groups this span belongs to
                for group in span_state.group_times.keys().cloned().collect::<Vec<_>>() {
                    // find the set of records related to these groups in the layer
                    if self.groups.get(&group).unwrap().records.contains(&name) {
                        // if we are a record for this group then increment the relevant
                        // span-local timing and continue to the next group
                        span_state.increment(&group, timing);
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
            if let Some(metrics_group) = self.groups.get(&name) {
                if let Some(t) = span_state.group_times.get(&name).filter(|&t| t.updated) {
                    (metrics_group.consumer)(*t);
                }
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
            updated: false,
        };
        let t2 = Timings {
            idle: 3,
            busy: 5,
            last: now,
            updated: false,
        };
        let t3 = t1 + t2;
        assert_eq!(
            t3,
            Timings {
                idle: 8,
                busy: 10,
                last: now,
                updated: false,
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
            updated: false,
        };
        let t2 = Timings {
            idle: 3,
            busy: 5,
            last: now,
            updated: false,
        };
        t1 += t2;
        assert_eq!(
            t1,
            Timings {
                idle: 8,
                busy: 10,
                last: now,
                updated: false,
            }
        )
    }

    #[test]
    fn span_state_increment() {
        let group = String::from("group");
        let mut span_state = SpanState::new(&group);
        let t1 = Timings {
            idle: 5,
            busy: 5,
            last: Instant::now(),
            updated: true,
        };
        span_state.increment(&group, t1);
        assert_eq!(span_state.group_times.get(&group).unwrap().idle, t1.idle);
        assert_eq!(span_state.group_times.get(&group).unwrap().busy, t1.busy);
    }

    #[test]
    fn metrics_layer() {
        let consumer = |_| println!("group/record");
        let ml = MetricsLayer::default().gather("group", consumer, vec!["record"]);
        assert_eq!(ml.groups.get("group").unwrap().records, vec!["record"]);
    }
}
