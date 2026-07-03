use crate::DEFAULT_CHUNK_CAP;

#[derive(Debug, Clone)]
pub struct Config {
    id: Option<u64>,
    chunk_cap: Option<usize>,
    segment_cost: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            id: None,
            chunk_cap: Some(DEFAULT_CHUNK_CAP),
            segment_cost: None,
        }
    }
}

impl Config {
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    pub fn id(&self) -> Option<u64> {
        self.id
    }

    pub fn chunk_cap(&self) -> Option<usize> {
        self.chunk_cap
    }

    pub fn segment_cost(&self) -> Option<usize> {
        self.segment_cost
    }
}

#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    config: Config,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            config: Config::default(),
        }
    }
}

impl ConfigBuilder {
    pub fn id(mut self, id: u64) -> Self {
        self.config.id = Some(id);
        self
    }

    pub fn chunk_cap(mut self, cap: Option<usize>) -> Self {
        self.config.chunk_cap = cap;
        self
    }

    pub fn segment_cost(mut self, cost: Option<usize>) -> Self {
        self.config.segment_cost = cost;
        self
    }

    pub fn build(self) -> Config {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_carries_standard_settings() {
        let config = Config::default();
        assert_eq!(config.id(), None);
        assert_eq!(config.chunk_cap(), Some(DEFAULT_CHUNK_CAP));
        assert_eq!(config.segment_cost(), None);
    }

    #[test]
    fn builder_overrides_only_set_fields() {
        let config = Config::builder().id(7).segment_cost(Some(5_000)).build();
        assert_eq!(config.id(), Some(7));
        assert_eq!(config.chunk_cap(), Some(DEFAULT_CHUNK_CAP));
        assert_eq!(config.segment_cost(), Some(5_000));
    }
}
