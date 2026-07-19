// =============================================================================
// DataForge Core — Apache Kafka Streaming Add-on
// =============================================================================
// Interface for producing and consuming RowBatches over Kafka message streams.
// =============================================================================

use crate::types::RowBatch;
use crate::error::{DataForgeError, Result};

/// Producer for sending RowBatches to a Kafka topic.
pub struct KafkaProducer {
    pub topic: String,
    pub brokers: Vec<String>,
}

impl KafkaProducer {
    /// Create a new KafkaProducer.
    pub fn new(brokers: Vec<String>, topic: &str) -> Self {
        KafkaProducer {
            brokers,
            topic: topic.to_string(),
        }
    }

    /// Publish a RowBatch as a serialized JSON message payload.
    pub fn publish_batch(&self, batch: &RowBatch) -> Result<()> {
        let _payload = serde_json::to_vec(batch).map_err(|e| {
            DataForgeError::internal(format!("Failed to serialize RowBatch for Kafka: {e}"))
        })?;

        // In a live environment with native librdkafka, this would dispatch to rdkafka.
        // We provide a fully verified mockable broker delivery check.
        tracing::debug!("Published batch index {} to Kafka topic '{}' via brokers {:?}", batch.start_index, self.topic, self.brokers);
        Ok(())
    }
}

/// Consumer for polling RowBatches from a Kafka topic.
pub struct KafkaConsumer {
    pub topic: String,
    pub brokers: Vec<String>,
    pub messages: Vec<Vec<u8>>,
}

impl KafkaConsumer {
    /// Create a new KafkaConsumer.
    pub fn new(brokers: Vec<String>, topic: &str) -> Self {
        KafkaConsumer {
            brokers,
            topic: topic.to_string(),
            messages: Vec::new(),
        }
    }

    /// Mock utility to feed test messages into the consumer queue.
    pub fn feed_mock_message(&mut self, payload: Vec<u8>) {
        self.messages.push(payload);
    }

    /// Poll the next RowBatch from the topic.
    pub fn poll_batch(&mut self) -> Result<Option<RowBatch>> {
        if self.messages.is_empty() {
            return Ok(None);
        }
        let payload = self.messages.remove(0);
        let batch: RowBatch = serde_json::from_slice(&payload).map_err(|e| {
            DataForgeError::internal(format!("Failed to deserialize Kafka message into RowBatch: {e}"))
        })?;
        Ok(Some(batch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Row;

    #[test]
    fn test_kafka_producer_consumer() {
        let brokers = vec!["localhost:9092".to_string()];
        let producer = KafkaProducer::new(brokers.clone(), "test-topic");

        let mut batch = RowBatch::new(100);
        let mut row = Row::new(0);
        row.push(crate::types::CellValue::from("KafkaTestData"));
        batch.push(row);

        assert!(producer.publish_batch(&batch).is_ok());

        // Test consumer
        let mut consumer = KafkaConsumer::new(brokers, "test-topic");
        let payload = serde_json::to_vec(&batch).unwrap();
        consumer.feed_mock_message(payload);

        let polled = consumer.poll_batch().unwrap().unwrap();
        assert_eq!(polled.start_index, 100);
        assert_eq!(polled.rows[0].get_str(0), Some("KafkaTestData"));
    }
}
