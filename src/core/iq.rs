#[derive(Debug, Clone, Copy)]
pub struct IqSample {
    pub i: f32,
    pub q: f32,
}

pub struct IqBlock {
    pub sequence: u64,
    pub timestamp: u64,
    pub center_frequency_hz: u64,
    pub sample_rate: u32,
    pub samples: Vec<IqSample>,
}

impl IqBlock {
    pub fn new(
        sequence: u64,
        timestamp: u64,
        center_frequency_hz: u64,
        sample_rate: u32,
        samples: Vec<IqSample>,
    ) -> Self {
        Self {
            sequence,
            timestamp,
            center_frequency_hz,
            sample_rate,
            samples,
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }
}

pub const IQ_BLOCK_SIZE: usize = 4096;

pub struct IqReblocker {
    buffer: Vec<IqSample>,
    sequence: u64,
}

impl IqReblocker {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(IQ_BLOCK_SIZE * 2),
            sequence: 0,
        }
    }

    pub fn push(
        &mut self,
        input: &[IqSample],
        timestamp: u64,
        center_frequency_hz: u64,
        sample_rate: u32,
    ) -> Vec<IqBlock> {
        self.buffer.extend_from_slice(input);

        let mut output = Vec::new();

        while self.buffer.len() >= IQ_BLOCK_SIZE {
            let samples: Vec<IqSample> = self.buffer.drain(..IQ_BLOCK_SIZE).collect();

            output.push(IqBlock::new(
                self.sequence,
                timestamp,
                center_frequency_hz,
                sample_rate,
                samples,
            ));

            self.sequence += 1;
        }

        output
    }

    pub fn pending_samples(&self) -> usize {
        self.buffer.len()
    }
}

pub struct IqDistributor {
    sinks: Vec<Box<dyn IqSink>>,
    blocks_distributed: u64,
}

impl Default for IqDistributor {
    fn default() -> Self {
        Self::new()
    }
}

impl IqDistributor {
    pub fn new() -> Self {
        Self {
            sinks: Vec::new(),
            blocks_distributed: 0,
        }
    }

    pub fn add_sink(&mut self, sink: Box<dyn IqSink>) {
        self.sinks.push(sink);
    }

    pub fn sink_count(&self) -> usize {
        self.sinks.len()
    }

    pub fn distribute(&mut self, block: &IqBlock) {
        for sink in &mut self.sinks {
            sink.push(block);
        }

        if !self.sinks.is_empty() {
            self.blocks_distributed += 1;
        }
    }

    pub fn blocks_distributed(&self) -> u64 {
        self.blocks_distributed
    }
}

pub trait IqSink: Send {
    fn push(&mut self, block: &IqBlock);
}

#[derive(Debug, Default)]
pub struct IqMonitorSink {
    blocks_received: u64,
    samples_received: u64,
}

impl IqMonitorSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn blocks_received(&self) -> u64 {
        self.blocks_received
    }

    pub fn samples_received(&self) -> u64 {
        self.samples_received
    }
}

impl IqSink for IqMonitorSink {
    fn push(&mut self, block: &IqBlock) {
        self.blocks_received += 1;
        self.samples_received += block.samples.len() as u64;
    }
}