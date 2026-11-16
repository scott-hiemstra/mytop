use std::collections::VecDeque;

/// Fixed-size ring buffer of samples used for the graphs.
pub struct History {
    buf: VecDeque<f64>,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
        }
    }

    pub fn push(&mut self, v: f64) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(v);
    }

    /// The newest `n` samples as chart points, right-aligned so the latest
    /// sample is always at x = n-1.
    pub fn points(&self, n: usize) -> Vec<(f64, f64)> {
        let len = self.buf.len();
        let take = len.min(n);
        let offset = n - take;
        self.buf
            .iter()
            .skip(len - take)
            .enumerate()
            .map(|(i, v)| ((offset + i) as f64, *v))
            .collect()
    }
}
