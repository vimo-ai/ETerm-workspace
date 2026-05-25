use base64::Engine;
use terminal_core::crosswords::{Crosswords, CrosswordsSize};
use terminal_core::event::{EventListener, RioEvent, WindowId};
use terminal_core::handler::Processor;
use terminal_core::mode::CursorShape;
use terminal_core::snapshot::GridSnapshot;

#[derive(Clone)]
pub struct DaemonEventListener;

impl EventListener for DaemonEventListener {
    fn event(&self) -> (Option<RioEvent>, bool) {
        (None, false)
    }

    fn send_event(&self, _event: RioEvent, _window_id: WindowId) {}
}

pub struct TerminalState {
    crosswords: Crosswords<DaemonEventListener>,
    processor: Processor,
}

impl TerminalState {
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = CrosswordsSize::new(cols as usize, rows as usize);
        let window_id = WindowId::from(0u64);
        let crosswords = Crosswords::new(
            size,
            CursorShape::Block,
            DaemonEventListener,
            window_id,
            0,
        );
        Self {
            crosswords,
            processor: Processor::new(),
        }
    }

    pub fn replay_history(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        eprintln!(
            "[daemon] replaying {} bytes of ring buffer history into parser",
            data.len()
        );
        self.processor.advance(&mut self.crosswords, data);
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.processor.advance(&mut self.crosswords, data);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = CrosswordsSize::new(cols as usize, rows as usize);
        self.crosswords.resize(size);
    }

    pub fn capture_snapshot(&self) -> Option<String> {
        let snapshot = GridSnapshot::capture(&self.crosswords);
        let bytes = snapshot.to_bytes();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Some(encoded)
    }
}
