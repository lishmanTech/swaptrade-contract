use crate::error::{MarketDataError, Result};
use crate::types::*;
use std::io::{Read, Write};
use lz4::block::{compress as lz4_compress, decompress as lz4_decompress, CompressionMode as Lz4CompressionMode};
use zstd::bulk::{compress as zstd_compress, decompress as zstd_decompress};
use bincode;
use serde::{Serialize, Deserialize};
use tracing::{debug, info};

pub struct CompressionEngine {
    lz4_level: i32,
    zstd_level: i32,
    compression_stats: parking_lot::RwLock<CompressionStats>,
}

#[derive(Debug, Default)]
pub struct CompressionStats {
    pub total_compressions: u64,
    pub total_decompressions: u64,
    pub total_original_bytes: u64,
    pub total_compressed_bytes: u64,
    pub average_compression_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedMarketData {
    pub compression_type: CompressionType,
    pub original_size: usize,
    pub compressed_size: usize,
    pub checksum: u32,
    pub data: Vec<u8>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl CompressionEngine {
    pub fn new() -> Self {
        Self {
            lz4_level: 4, // Balanced compression level
            zstd_level: 3, // Balanced compression level
            compression_stats: parking_lot::RwLock::new(CompressionStats::default()),
        }
    }

    pub fn with_levels(lz4_level: i32, zstd_level: i32) -> Self {
        Self {
            lz4_level: lz4_level.max(0).min(16),
            zstd_level: zstd_level.max(1).min(22),
            compression_stats: parking_lot::RwLock::new(CompressionStats::default()),
        }
    }

    pub fn compress(&self, data: &[u8], compression_type: CompressionType) -> Result<Vec<u8>> {
        let start_time = std::time::Instant::now();
        
        let compressed_data = match compression_type {
            CompressionType::None => data.to_vec(),
            CompressionType::LZ4 => self.compress_lz4(data)?,
            CompressionType::Zstd => self.compress_zstd(data)?,
        };

        let compression_time = start_time.elapsed();
        let compression_ratio = compressed_data.len() as f64 / data.len() as f64;

        // Update statistics
        {
            let mut stats = self.compression_stats.write();
            stats.total_compressions += 1;
            stats.total_original_bytes += data.len() as u64;
            stats.total_compressed_bytes += compressed_data.len() as u64;
            stats.average_compression_ratio = stats.total_compressed_bytes as f64 / stats.total_original_bytes as f64;
        }

        debug!("Compressed {} bytes to {} bytes using {:?} in {:?} (ratio: {:.2}%)",
               data.len(), compressed_data.len(), compression_type, compression_time, compression_ratio * 100.0);

        Ok(compressed_data)
    }

    pub fn decompress(&self, compressed_data: &[u8]) -> Result<Vec<u8>> {
        let start_time = std::time::Instant::now();

        // Try to detect compression type from the data
        let compression_type = self.detect_compression_type(compressed_data)?;
        
        let decompressed_data = match compression_type {
            CompressionType::None => compressed_data.to_vec(),
            CompressionType::LZ4 => self.decompress_lz4(compressed_data)?,
            CompressionType::Zstd => self.decompress_zstd(compressed_data)?,
        };

        let decompression_time = start_time.elapsed();

        // Update statistics
        {
            let mut stats = self.compression_stats.write();
            stats.total_decompressions += 1;
        }

        debug!("Decompressed {} bytes to {} bytes using {:?} in {:?}",
               compressed_data.len(), decompressed_data.len(), compression_type, decompression_time);

        Ok(decompressed_data)
    }

    pub fn compress_market_data(&self, market_data: &MarketData, compression_type: CompressionType) -> Result<CompressedMarketData> {
        let serialized = bincode::serialize(market_data)?;
        let compressed = self.compress(&serialized, compression_type.clone())?;
        let checksum = crc::crc32::checksum_ieee(&compressed);

        Ok(CompressedMarketData {
            compression_type,
            original_size: serialized.len(),
            compressed_size: compressed.len(),
            checksum,
            data: compressed,
            timestamp: chrono::Utc::now(),
        })
    }

    pub fn decompress_market_data(&self, compressed_data: &CompressedMarketData) -> Result<MarketData> {
        // Verify checksum
        let calculated_checksum = crc::crc32::checksum_ieee(&compressed_data.data);
        if calculated_checksum != compressed_data.checksum {
            return Err(MarketDataError::DataIntegrityFailed);
        }

        let decompressed = self.decompress(&compressed_data.data)?;
        let market_data: MarketData = bincode::deserialize(&decompressed)?;
        
        Ok(market_data)
    }

    fn compress_lz4(&self, data: &[u8]) -> Result<Vec<u8>> {
        let compressed = lz4_compress(
            data,
            None, // Use default dictionary
            false, // No content checksum
            Lz4CompressionMode::HighCompression(self.lz4_level),
            false, // No independent blocks
        ).map_err(|e| MarketDataError::Compression(format!("LZ4 compression failed: {}", e)))?;

        Ok(compressed)
    }

    fn compress_zstd(&self, data: &[u8]) -> Result<Vec<u8>> {
        let compressed = zstd_compress(data, self.zstd_level)
            .map_err(|e| MarketDataError::Compression(format!("Zstd compression failed: {}", e)))?;
        Ok(compressed)
    }

    fn decompress_lz4(&self, compressed_data: &[u8]) -> Result<Vec<u8>> {
        let decompressed = lz4_decompress(compressed_data, None)
            .map_err(|e| MarketDataError::Compression(format!("LZ4 decompression failed: {}", e)))?;
        Ok(decompressed)
    }

    fn decompress_zstd(&self, compressed_data: &[u8]) -> Result<Vec<u8>> {
        let decompressed = zstd_decompress(compressed_data, 10 * 1024 * 1024) // 10MB max size
            .map_err(|e| MarketDataError::Compression(format!("Zstd decompression failed: {}", e)))?;
        Ok(decompressed)
    }

    fn detect_compression_type(&self, data: &[u8]) -> Result<CompressionType> {
        if data.is_empty() {
            return Ok(CompressionType::None);
        }

        // LZ4 magic number detection
        if data.len() >= 4 {
            let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            if magic == 0x04224D18 || magic == 0x184D2204 {
                return Ok(CompressionType::LZ4);
            }
        }

        // Zstd magic number detection
        if data.len() >= 4 {
            let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            if magic == 0xFD2FB528 {
                return Ok(CompressionType::Zstd);
            }
        }

        // If no magic number detected, assume uncompressed
        Ok(CompressionType::None)
    }

    pub fn get_compression_stats(&self) -> CompressionStats {
        self.compression_stats.read().clone()
    }

    pub fn benchmark_compression(&self, data: &[u8]) -> Result<CompressionBenchmark> {
        let mut results = Vec::new();

        // Benchmark no compression
        let start = std::time::Instant::now();
        let uncompressed_size = data.len();
        let no_compression_time = start.elapsed();

        results.push(CompressionResult {
            compression_type: CompressionType::None,
            original_size: uncompressed_size,
            compressed_size: uncompressed_size,
            compression_ratio: 1.0,
            compression_time: no_compression_time,
            decompression_time: no_compression_time,
        });

        // Benchmark LZ4
        let start = std::time::Instant::now();
        let lz4_compressed = self.compress_lz4(data)?;
        let lz4_compression_time = start.elapsed();

        let start = std::time::Instant::now();
        let _lz4_decompressed = self.decompress_lz4(&lz4_compressed)?;
        let lz4_decompression_time = start.elapsed();

        results.push(CompressionResult {
            compression_type: CompressionType::LZ4,
            original_size: uncompressed_size,
            compressed_size: lz4_compressed.len(),
            compression_ratio: lz4_compressed.len() as f64 / uncompressed_size as f64,
            compression_time: lz4_compression_time,
            decompression_time: lz4_decompression_time,
        });

        // Benchmark Zstd
        let start = std::time::Instant::now();
        let zstd_compressed = self.compress_zstd(data)?;
        let zstd_compression_time = start.elapsed();

        let start = std::time::Instant::now();
        let _zstd_decompressed = self.decompress_zstd(&zstd_compressed)?;
        let zstd_decompression_time = start.elapsed();

        results.push(CompressionResult {
            compression_type: CompressionType::Zstd,
            original_size: uncompressed_size,
            compressed_size: zstd_compressed.len(),
            compression_ratio: zstd_compressed.len() as f64 / uncompressed_size as f64,
            compression_time: zstd_compression_time,
            decompression_time: zstd_decompression_time,
        });

        Ok(CompressionBenchmark { results })
    }

    pub fn optimize_for_bandwidth(&self, target_compression_ratio: f64) -> CompressionType {
        let stats = self.compression_stats.read();
        let current_ratio = stats.average_compression_ratio;

        if current_ratio <= target_compression_ratio {
            CompressionType::LZ4 // Fast compression, good ratio
        } else {
            CompressionType::Zstd // Better compression for bandwidth savings
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub compression_type: CompressionType,
    pub original_size: usize,
    pub compressed_size: usize,
    pub compression_ratio: f64,
    pub compression_time: std::time::Duration,
    pub decompression_time: std::time::Duration,
}

#[derive(Debug)]
pub struct CompressionBenchmark {
    pub results: Vec<CompressionResult>,
}

impl CompressionBenchmark {
    pub fn best_for_speed(&self) -> &CompressionResult {
        self.results
            .iter()
            .min_by_key(|r| r.compression_time + r.decompression_time)
            .unwrap()
    }

    pub fn best_for_size(&self) -> &CompressionResult {
        self.results
            .iter()
            .min_by_key(|r| r.compressed_size)
            .unwrap()
    }

    pub fn best_balanced(&self) -> &CompressionResult {
        self.results
            .iter()
            .min_by_key(|r| {
                let size_score = r.compression_ratio;
                let speed_score = (r.compression_time + r.decompression_time).as_nanos() as f64;
                size_score * speed_score
            })
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MarketData, MarketDataType, TradeData, OrderSide};

    #[test]
    fn test_compression_roundtrip() {
        let engine = CompressionEngine::new();
        let market_data = MarketData {
            symbol: "BTCUSDT".to_string(),
            timestamp: chrono::Utc::now(),
            data_type: MarketDataType::Trade(TradeData {
                price: 50000.0,
                quantity: 0.1,
                side: OrderSide::Buy,
                trade_id: "12345".to_string(),
            }),
            sequence: 1,
            checksum: 0,
        };

        let compressed = engine.compress_market_data(&market_data, CompressionType::LZ4).unwrap();
        let decompressed = engine.decompress_market_data(&compressed).unwrap();

        assert_eq!(market_data.symbol, decompressed.symbol);
        assert_eq!(market_data.sequence, decompressed.sequence);
    }

    #[test]
    fn test_compression_ratio() {
        let engine = CompressionEngine::new();
        let data = vec![0u8; 1024]; // 1KB of zeros

        let compressed = engine.compress(&data, CompressionType::LZ4).unwrap();
        let ratio = compressed.len() as f64 / data.len() as f64;

        assert!(ratio < 0.5, "Compression ratio should be better than 50%");
    }
}
