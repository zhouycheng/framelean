use std::collections::VecDeque;

use framelean_core::Result;

use crate::Task;

pub trait Scheduler {
    fn enqueue(&mut self, task: Task) -> Result<()>;
    fn dequeue(&mut self) -> Option<Task>;
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Default)]
pub struct FifoScheduler {
    tasks: VecDeque<Task>,
}

impl FifoScheduler {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Scheduler for FifoScheduler {
    fn enqueue(&mut self, mut task: Task) -> Result<()> {
        task.queue()?;
        self.tasks.push_back(task);
        Ok(())
    }

    fn dequeue(&mut self) -> Option<Task> {
        self.tasks.pop_front()
    }

    fn len(&self) -> usize {
        self.tasks.len()
    }
}
