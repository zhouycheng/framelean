use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};
struct State {
    seq: u64,
    ring: VecDeque<Value>,
    active: Value,
    lifecycle: String,
    health: Value,
    last_progress: Instant,
}
pub struct Events {
    pub instance: String,
    state: Mutex<State>,
    changed: Condvar,
}
impl Events {
    pub fn new() -> Self {
        Self {
            instance: crate::uid(),
            state: Mutex::new(State {
                seq: 0,
                ring: VecDeque::new(),
                active: Value::Null,
                lifecycle: "starting".into(),
                health: Value::Null,
                last_progress: Instant::now() - Duration::from_secs(1),
            }),
            changed: Condvar::new(),
        }
    }
    fn append(&self, s: &mut State, kind: &str, data: Value) {
        s.seq += 1;
        let event = json!({"event_id":format!("{}:{}",self.instance,s.seq),"instance_id":self.instance,"seq":s.seq,"type":kind,"data":data});
        s.ring.push_back(event);
        while s.ring.len() > 1024 {
            s.ring.pop_front();
        }
        self.changed.notify_all();
    }
    pub fn state_changed(&self, revision: i64) {
        let mut s = self.state.lock().unwrap();
        self.append(&mut s, "state_changed", json!({"revision":revision}));
    }
    pub fn progress(&self, id: &str, pid: u32, event: Value) {
        let mut s = self.state.lock().unwrap();
        s.active = json!({"id":id,"host_pid":pid,"event":event});
        if s.last_progress.elapsed() >= Duration::from_millis(250) {
            s.last_progress = Instant::now();
            let value = s.active.clone();
            self.append(&mut s, "progress", value);
        }
    }
    pub fn clear(&self) {
        let mut s = self.state.lock().unwrap();
        s.active = Value::Null;
        self.append(&mut s, "progress", Value::Null);
    }
    pub fn lifecycle(&self, status: &str, health: Value) {
        let mut s = self.state.lock().unwrap();
        s.lifecycle = status.into();
        s.health = health;
        self.append(&mut s, "lifecycle", json!({"state":status}));
    }
    pub fn snapshot(&self) -> Value {
        let s = self.state.lock().unwrap();
        json!({"protocol_version":fll_contracts::HTTP_VERSION,"instance_id":self.instance,"seq":s.seq,"event_id":format!("{}:{}",self.instance,s.seq),"active":s.active,"lifecycle":s.lifecycle,"health":s.health})
    }
    pub fn after(&self, cursor: Option<&str>) -> Option<Vec<Value>> {
        let s = self.state.lock().unwrap();
        let cursor = cursor
            .and_then(|c| c.rsplit_once(':'))
            .filter(|(i, _)| *i == self.instance)
            .and_then(|(_, n)| n.parse::<u64>().ok())?;
        let oldest = s
            .ring
            .front()
            .and_then(|v| v["seq"].as_u64())
            .unwrap_or(s.seq + 1);
        if cursor > s.seq || cursor + 1 < oldest {
            return None;
        }
        Some(
            s.ring
                .iter()
                .filter(|v| v["seq"].as_u64().unwrap_or(0) > cursor)
                .cloned()
                .collect(),
        )
    }
    pub fn wait(&self) {
        let guard = self.state.lock().unwrap();
        drop(
            self.changed
                .wait_timeout(guard, Duration::from_secs(2))
                .unwrap(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursors_reset_on_gaps_or_new_sessions_and_progress_is_coalesced() {
        let events = Events::new();
        let old = events.snapshot()["event_id"].as_str().unwrap().to_owned();
        for revision in 0..1030 {
            events.state_changed(revision);
        }
        assert!(events.after(Some(&old)).is_none());
        assert!(events.after(Some("different:1030")).is_none());
        let current = events.snapshot()["event_id"].as_str().unwrap().to_owned();
        for n in 0..1000 {
            events.progress("attempt", 10, json!({"progress":n}));
        }
        assert!(events.after(Some(&current)).unwrap().len() <= 2);
        assert_eq!(events.snapshot()["active"]["event"]["progress"], 999);
        events.clear();
        assert!(events.snapshot()["active"].is_null());
    }
}
