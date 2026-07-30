#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_symphonia_decoder_new() {
        let mut codec_registry = symphonia::core::codecs::CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut codec_registry);
        let decoder = SymphoniaDecoder::new(codec_registry);
        // Decoder creation test - we can't test much more without actual audio files
    }
    
    #[test]
    fn test_lofty_metadata_reader_new() {
        let reader = LoftyMetadataReader::new();
        // Reader creation test
    }
    
    #[test]
    fn test_image_cover_loader_new() {
        let loader = ImageCoverLoader::new();
        // Loader creation test
    }
    
    #[test]
    fn test_audio_file_scanner_new() {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let scanner = AudioFileScanner::new(cancel_flag);
        // Scanner creation test
    }
    
    #[test]
    fn test_filesystem_watcher_new() {
        // This test might fail if there are filesystem permissions issues
        let (tx, _) = crossbeam_channel::unbounded();
        let result = FilesystemWatcher::new(tx);
        // The result could be Ok or Err depending on the system
    }
}