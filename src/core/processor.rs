use super::{IqBlock, IqDistributor};

#[derive(Debug, Default, Clone, Copy)]
pub struct IqProcessorStats {
    pub blocks_processed: u64,
    pub samples_processed: u64,
    pub last_sequence: u64,
    pub last_timestamp: u64,
    pub center_frequency_hz: u64,
    pub sample_rate: u32,
}

#[derive(Default)]
pub struct IqProcessor {
    stats: IqProcessorStats,
    distributor: IqDistributor,
}

impl IqProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, block: IqBlock) {
        self.stats.blocks_processed += 1;
        self.stats.samples_processed += block.samples.len() as u64;

        self.stats.last_sequence = block.sequence;
        self.stats.last_timestamp = block.timestamp;
        self.stats.center_frequency_hz = block.center_frequency_hz;
        self.stats.sample_rate = block.sample_rate;

        self.distributor.distribute(&block);
    }

    pub fn distributor_mut(&mut self) -> &mut IqDistributor {
        &mut self.distributor
    }

    pub fn blocks_processed(&self) -> u64 {
        self.stats.blocks_processed
    }

    pub fn samples_processed(&self) -> u64 {
        self.stats.samples_processed
    }
}
