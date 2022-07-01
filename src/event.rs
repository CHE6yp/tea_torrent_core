use std::fmt::Debug;
use std::fmt::Formatter;
use std::sync::Arc;

pub struct Event<T>
where
    T: Copy,
{
    subscribers: Vec<Arc<dyn Fn(T) + 'static + Send + Sync>>,
}

impl<T: Copy> Debug for Event<T> {
    // add code here
    fn fmt(&self, f: &mut Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "{}", self.subscribers.len())
    }
}

impl<T: Copy> Event<T> {
    pub fn new() -> Event<T> {
        Event {
            subscribers: vec![],
        }
    }

    pub fn emit(&self, arg: T) {
        self.subscribers.iter().for_each(|e| e(arg));
    }

    pub fn subscribe(&mut self, callback: Arc<dyn Fn(T) + 'static + Send + Sync>) {
        self.subscribers.push(callback);
    }
}
