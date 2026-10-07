use std::{io, path::Path};

use async_compression::{
    Level,
    tokio::bufread::{ZstdDecoder, ZstdEncoder},
    zstd::{CParameter, DParameter},
};
use bitcode::{Decode, Encode};
use eyre::WrapErr;
use tokio::{
    fs::File,
    io::{
        AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader,
        SeekFrom,
    },
};

use crate::{CoreConfig, Footer, SourceManifest, validate_file_target};

const BUFFER_SIZE: usize = 64 * 1024;
// Payloads may be arbitrarily large; metadata has a separate bounded allocation.
const MAX_MANIFEST_SIZE: u64 = 64 * 1024 * 1024;

/// Metadata for one independently compressed zstd frame in an installer.
#[derive(Encode, Decode, PartialEq, Debug, Clone)]
pub struct PackageEntry {
    pub name: String,
    /// Absolute byte offset from the beginning of the installer executable.
    pub offset: u64,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
}

/// Stored after the compressed payloads, immediately before the footer.
#[derive(Encode, Decode, PartialEq, Debug, Clone)]
pub struct PackageManifest {
    pub core_config: CoreConfig,
    /// Beginning of the payload area, immediately after the bootstrap executable.
    pub payload_offset: u64,
    pub entries: Vec<PackageEntry>,
}

impl SourceManifest {
    /// Streams source files into individual zstd frames, then writes bitcode metadata.
    /// `payload_offset` is the writer's current absolute installer position.
    /// Returns the manifest offset for the footer. The caller writes the footer and flushes.
    /// This never shuts down the supplied writer or buffers an entire source file.
    pub async fn encode<W: AsyncWrite + Unpin + ?Sized>(
        &self,
        writer: &mut W,
        payload_offset: u64,
    ) -> eyre::Result<u64> {
        self.validate_main_executable()?;
        let mut position = payload_offset;
        let mut package = PackageManifest {
            core_config: self.core_config.clone(),
            payload_offset,
            entries: Vec::with_capacity(self.entries.len()),
        };
        for entry in &self.entries {
            let source = File::open(&entry.source_path).await.wrap_err_with(|| {
                format!("Failed to open source file {}", entry.source_path.display())
            })?;
            let offset = position;
            let uncompressed_size = compress_file(source, writer, &mut position)
                .await
                .wrap_err_with(|| format!("Failed to compress packaged file {}", entry.name))?;
            package.entries.push(PackageEntry {
                name: entry.name.clone(),
                offset,
                compressed_size: position - offset,
                uncompressed_size,
            });
        }
        let manifest_offset = position;
        let metadata = bitcode::encode(&package);
        let metadata_size = u64::try_from(metadata.len())?;
        if metadata_size > MAX_MANIFEST_SIZE {
            eyre::bail!("Package metadata exceeds the 64 MiB manifest limit");
        }
        position
            .checked_add(metadata_size)
            .ok_or_else(|| eyre::eyre!("Installer size exceeds u64 offsets"))?;
        writer
            .write_all(&metadata)
            .await
            .wrap_err("Failed to write package manifest")?;
        Ok(manifest_offset)
    }
}

impl PackageManifest {
    /// Seeks to the footer and reads only metadata. Payloads remain in the reader.
    /// The reader must contain the complete installer, including its footer.
    pub async fn decode<R: AsyncRead + AsyncSeek + Unpin + ?Sized>(
        reader: &mut R,
    ) -> eyre::Result<Self> {
        let file_size = reader.seek(SeekFrom::End(0)).await?;
        let footer_start = file_size
            .checked_sub(Footer::SIZE as u64)
            .ok_or_else(|| eyre::eyre!("Installer is too small to contain a footer"))?;
        reader.seek(SeekFrom::Start(footer_start)).await?;
        let mut footer_bytes = [0u8; Footer::SIZE];
        reader.read_exact(&mut footer_bytes).await?;
        if !footer_bytes.starts_with(&Footer::MAGIC.to_le_bytes()) {
            eyre::bail!("Installer footer has invalid magic bytes");
        }
        let footer = Footer::try_from(footer_bytes.as_slice())
            .map_err(|_| eyre::eyre!("Failed to read installer footer"))?;
        if footer.version != Footer::FORMAT_VERSION {
            eyre::bail!("Unsupported installer format version {}", footer.version);
        }
        let manifest_size = footer_start
            .checked_sub(footer.manifest_offset)
            .ok_or_else(|| eyre::eyre!("Invalid manifest byte range"))?;
        if manifest_size > MAX_MANIFEST_SIZE {
            eyre::bail!("Package metadata exceeds the 64 MiB manifest limit");
        }
        reader.seek(SeekFrom::Start(footer.manifest_offset)).await?;
        let mut metadata = Vec::new();
        metadata.try_reserve_exact(usize::try_from(manifest_size)?)?;
        reader
            .take(manifest_size)
            .read_to_end(&mut metadata)
            .await?;
        if u64::try_from(metadata.len())? != manifest_size {
            eyre::bail!("Truncated package manifest");
        }
        let manifest: Self =
            bitcode::decode(&metadata).wrap_err("Failed to decode package manifest")?;
        manifest.validate_ranges(footer.manifest_offset)?;
        manifest.validate_main_executable()?;
        Ok(manifest)
    }

    pub fn validate_main_executable(&self) -> eyre::Result<()> {
        if let Some(target) = &self.core_config.main_executable {
            self.validate_file_target(Path::new(target))?;
        }
        Ok(())
    }

    pub fn validate_file_target(&self, target: &Path) -> eyre::Result<()> {
        validate_file_target(target, self.entries.iter().map(|entry| entry.name.as_str()))
    }

    fn validate_ranges(&self, manifest_offset: u64) -> eyre::Result<()> {
        if self.payload_offset > manifest_offset {
            eyre::bail!("Payload offset is beyond the manifest");
        }
        let mut next_offset = self.payload_offset;
        for entry in &self.entries {
            if entry.offset != next_offset || entry.compressed_size == 0 {
                eyre::bail!("Invalid payload byte range for {}", entry.name);
            }
            next_offset = entry
                .offset
                .checked_add(entry.compressed_size)
                .ok_or_else(|| eyre::eyre!("Payload byte range overflows for {}", entry.name))?;
            if next_offset > manifest_offset {
                eyre::bail!("Payload for {} extends into the manifest", entry.name);
            }
        }
        if next_offset != manifest_offset {
            eyre::bail!("Payload ranges do not cover the payload area");
        }
        Ok(())
    }
}

impl PackageEntry {
    /// Streams one bounded payload range to the supplied writer, validating its
    /// zstd frame, checksum, and declared size. The caller flushes the writer.
    pub async fn extract<R, W>(&self, reader: &mut R, writer: &mut W) -> eyre::Result<()>
    where
        R: AsyncRead + AsyncSeek + Unpin + ?Sized,
        W: AsyncWrite + Unpin + ?Sized,
    {
        if self.compressed_size == 0 || self.offset.checked_add(self.compressed_size).is_none() {
            eyre::bail!("Invalid payload byte range for {}", self.name);
        }
        reader.seek(SeekFrom::Start(self.offset)).await?;
        let input = BufReader::with_capacity(BUFFER_SIZE, reader.take(self.compressed_size));
        let mut decoder = ZstdDecoder::with_params(input, &[DParameter::window_log_max(27)]);
        decoder.multiple_members(false);
        let mut remaining = self.uncompressed_size;
        let mut buffer = vec![0u8; BUFFER_SIZE];
        while remaining != 0 {
            let limit = remaining.min(BUFFER_SIZE as u64) as usize;
            let count = decoder.read(&mut buffer[..limit]).await?;
            if count == 0 {
                eyre::bail!("Decompressed file is shorter than its declared size");
            }
            writer.write_all(&buffer[..count]).await?;
            remaining -= count as u64;
        }
        // Reading to frame EOF validates the trailer/checksum and catches excess output.
        if decoder.read(&mut buffer[..1]).await? != 0 {
            eyre::bail!("Decompressed file exceeds its declared size");
        }
        let input = decoder.into_inner();
        if !input.buffer().is_empty() || input.get_ref().limit() != 0 {
            eyre::bail!("Compressed file contains data after its zstd frame");
        }
        Ok(())
    }
}

async fn compress_file<W: AsyncWrite + Unpin + ?Sized>(
    input: File,
    output: &mut W,
    position: &mut u64,
) -> io::Result<u64> {
    let mut encoder = ZstdEncoder::with_quality_and_params(
        BufReader::with_capacity(BUFFER_SIZE, input),
        Level::Default,
        &[CParameter::checksum_flag(true)],
    );
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        let count = encoder.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let next_position = position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("Installer size exceeds u64 offsets"))?;
        output.write_all(&buffer[..count]).await?;
        *position = next_position;
    }
    // EOF completes the frame. The consumed source position gives its actual size,
    // including changes made after collecting source paths, without closing output.
    encoder.into_inner().stream_position().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InstallTarget, SourceEntry};
    use std::{
        io::Cursor,
        path::PathBuf,
        pin::Pin,
        sync::atomic::{AtomicU64, Ordering},
        task::{Context, Poll},
    };
    use tokio::io::ReadBuf;

    const BOOTSTRAP: &[u8] = b"fake installer executable";
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let temp = std::env::temp_dir();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = temp.join(format!(
                "simp-stream-test-{}-{timestamp}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            // Only remove the uniquely named directory created and owned by this test.
            if self.0.parent() == Some(std::env::temp_dir().as_path())
                && self
                    .0
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("simp-stream-test-"))
            {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn config() -> CoreConfig {
        CoreConfig {
            name: "Example".into(),
            version: "1.0".into(),
            description: None,
            eula: None,
            main_executable: None,
            auto_start: None,
            desktop_icon: None,
            start_menu: None,
            add_to_path: None,
            path_scope: None,
            target: InstallTarget::AppData,
        }
    }

    async fn sources(directory: &TestDirectory) -> SourceManifest {
        let names = ["empty.txt", "repeated.bin", "日本語.bin"];
        tokio::fs::write(directory.0.join(names[0]), [])
            .await
            .unwrap();
        let mut repeated = File::create(directory.0.join(names[1])).await.unwrap();
        let block = vec![42u8; BUFFER_SIZE];
        for _ in 0..16 {
            repeated.write_all(&block).await.unwrap();
        }
        repeated.flush().await.unwrap();
        tokio::fs::write(directory.0.join(names[2]), (0..=255u8).collect::<Vec<_>>())
            .await
            .unwrap();
        SourceManifest {
            core_config: config(),
            entries: names
                .into_iter()
                .map(|name| SourceEntry::new(&directory.0.join(name)).unwrap())
                .collect(),
        }
    }

    async fn encode(source: &SourceManifest) -> (Vec<u8>, u64) {
        let mut bytes = BOOTSTRAP.to_vec();
        let offset = source
            .encode(&mut bytes, BOOTSTRAP.len() as u64)
            .await
            .unwrap();
        footer(&mut bytes, offset);
        (bytes, offset)
    }
    fn footer(bytes: &mut Vec<u8>, offset: u64) {
        bytes.extend(<[u8; Footer::SIZE]>::from(Footer {
            version: Footer::FORMAT_VERSION,
            manifest_offset: offset,
        }));
    }
    fn replace_metadata(
        bytes: &mut Vec<u8>,
        offset: u64,
        change: impl FnOnce(&mut PackageManifest),
    ) {
        let start = offset as usize;
        let mut package: PackageManifest =
            bitcode::decode(&bytes[start..bytes.len() - Footer::SIZE]).unwrap();
        change(&mut package);
        bytes.truncate(start);
        bytes.extend(bitcode::encode(&package));
        footer(bytes, offset);
    }

    struct MetadataReader {
        inner: Cursor<Vec<u8>>,
        metadata_start: u64,
    }
    impl AsyncRead for MetadataReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.inner.position() < self.metadata_start {
                return Poll::Ready(Err(io::Error::other(
                    "Package loading must not read payloads",
                )));
            }
            Pin::new(&mut self.inner).poll_read(cx, buffer)
        }
    }
    impl AsyncSeek for MetadataReader {
        fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
            Pin::new(&mut self.inner).start_seek(position)
        }
        fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
            Pin::new(&mut self.inner).poll_complete(cx)
        }
    }

    #[tokio::test]
    async fn loads_only_metadata_and_extracts_independent_frames() {
        let directory = TestDirectory::new();
        let source = sources(&directory).await;
        let (bytes, offset) = encode(&source).await;
        let mut reader = MetadataReader {
            inner: Cursor::new(bytes.clone()),
            metadata_start: offset,
        };
        let package = PackageManifest::decode(&mut reader).await.unwrap();
        assert_eq!(package.core_config, source.core_config);
        assert_eq!(package.payload_offset, BOOTSTRAP.len() as u64);
        assert!(bytes.len() - (offset as usize) < 1024);
        assert!(package.entries[1].compressed_size < 1024);
        for (entry, input) in package.entries.iter().zip(source.entries) {
            assert_eq!(entry.name, input.name);
            let original = tokio::fs::read(input.source_path).await.unwrap();
            assert_eq!(entry.uncompressed_size, original.len() as u64);
            let start = entry.offset as usize;
            let end = start + entry.compressed_size as usize;
            assert_eq!(
                zstd::stream::decode_all(&bytes[start..end]).unwrap(),
                original
            );
            let mut output = Vec::new();
            entry
                .extract(&mut Cursor::new(&bytes), &mut output)
                .await
                .unwrap();
            assert_eq!(output, original);
        }
    }

    #[tokio::test]
    async fn builds_source_metadata_without_reading_file_contents() {
        let directory = TestDirectory::new();
        let mut configuration: crate::SimpConfig =
            serde_json::from_str(include_str!("../../examples/hello-world/simp.json")).unwrap();
        tokio::fs::write(directory.0.join("hello.cmd"), b"before")
            .await
            .unwrap();
        configuration.components = vec![crate::InstallComponent {
            component_path: directory.0.clone(),
        }];
        let source = SourceManifest::new(&configuration).await.unwrap();
        assert_eq!(source.entries.len(), 1);
        tokio::fs::write(
            directory.0.join("hello.cmd"),
            b"changed after collecting sources",
        )
        .await
        .unwrap();
        let (bytes, _) = encode(&source).await;
        let package = PackageManifest::decode(&mut Cursor::new(&bytes))
            .await
            .unwrap();
        let mut output = Vec::new();
        package.entries[0]
            .extract(&mut Cursor::new(&bytes), &mut output)
            .await
            .unwrap();
        assert_eq!(output, b"changed after collecting sources");
    }

    struct PartialWriter {
        bytes: Vec<u8>,
        pending_next: bool,
        shutdowns: usize,
    }
    impl AsyncWrite for PartialWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.pending_next {
                self.pending_next = false;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            self.pending_next = true;
            let count = bytes.len().min(7);
            self.bytes.extend_from_slice(&bytes[..count]);
            Poll::Ready(Ok(count))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.shutdowns += 1;
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn handles_pending_partial_writes_and_leaves_output_open() {
        let directory = TestDirectory::new();
        let source = sources(&directory).await;
        let mut writer = PartialWriter {
            bytes: BOOTSTRAP.to_vec(),
            pending_next: true,
            shutdowns: 0,
        };
        let offset = source
            .encode(
                &mut writer as &mut (dyn AsyncWrite + Unpin),
                BOOTSTRAP.len() as u64,
            )
            .await
            .unwrap();
        assert_eq!(writer.shutdowns, 0);
        footer(&mut writer.bytes, offset);
        let package = PackageManifest::decode(&mut Cursor::new(&writer.bytes))
            .await
            .unwrap();
        assert_eq!(package.entries.len(), 3);
        for entry in &package.entries {
            entry
                .extract(&mut Cursor::new(&writer.bytes), &mut tokio::io::sink())
                .await
                .unwrap();
        }
    }

    struct FailingWriter {
        remaining: usize,
    }
    impl AsyncWrite for FailingWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.remaining == 0 {
                return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
            }
            let count = bytes.len().min(self.remaining);
            self.remaining -= count;
            Poll::Ready(Ok(count))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn propagates_source_frame_and_metadata_errors_and_offset_overflow() {
        let directory = TestDirectory::new();
        let mut source = sources(&directory).await;
        let error = source
            .encode(&mut FailingWriter { remaining: 0 }, 0)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::BrokenPipe
        );
        // With no payloads, the first write is guaranteed to be the metadata.
        let metadata_only = SourceManifest {
            core_config: config(),
            entries: Vec::new(),
        };
        let error = metadata_only
            .encode(&mut FailingWriter { remaining: 0 }, 0)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Failed to write package manifest")
        );
        let mut bytes = Vec::new();
        assert!(source.encode(&mut bytes, u64::MAX).await.is_err());
        assert!(bytes.is_empty());
        source.entries[0].source_path = directory.0.join("missing");
        assert!(
            source
                .encode(&mut bytes, 0)
                .await
                .unwrap_err()
                .to_string()
                .contains("Failed to open source")
        );
        assert!(bytes.is_empty());
    }

    #[tokio::test]
    async fn rejects_invalid_ranges_and_application_targets_in_metadata() {
        let directory = TestDirectory::new();
        let (original, offset) = encode(&sources(&directory).await).await;
        for case in 0..9 {
            let mut bytes = original.clone();
            replace_metadata(&mut bytes, offset, |p| match case {
                0 => p.payload_offset = u64::MAX,
                1 => p.entries[0].offset = 0,
                2 => p.entries[0].compressed_size = 0,
                3 => p.entries[0].compressed_size = u64::MAX,
                4 => p.entries[1].offset = p.entries[0].offset,
                5 => p.entries[1].offset += 1,
                6 => p.entries.last_mut().unwrap().compressed_size += 1,
                7 => {
                    p.entries.pop();
                }
                _ => p.core_config.main_executable = Some("../missing.exe".into()),
            });
            assert!(
                PackageManifest::decode(&mut Cursor::new(bytes))
                    .await
                    .is_err(),
                "case {case}"
            );
        }
    }

    #[tokio::test]
    async fn detects_bad_sizes_and_corruption_during_extraction() {
        let directory = TestDirectory::new();
        let (original, offset) = encode(&sources(&directory).await).await;
        for size in [0, 255, 257, u64::MAX] {
            let mut bytes = original.clone();
            replace_metadata(&mut bytes, offset, |p| {
                p.entries[2].uncompressed_size = size
            });
            let package = PackageManifest::decode(&mut Cursor::new(&bytes))
                .await
                .unwrap();
            let mut output = Vec::new();
            assert!(
                package.entries[2]
                    .extract(&mut Cursor::new(&bytes), &mut output)
                    .await
                    .is_err()
            );
            assert!(output.len() as u64 <= size);
        }
        let mut bytes = original;
        let package = PackageManifest::decode(&mut Cursor::new(&bytes))
            .await
            .unwrap();
        let entry = &package.entries[2];
        bytes[(entry.offset + entry.compressed_size - 1) as usize] ^= 1;
        // Loading stays cheap and does not decompress; extraction checks the frame.
        PackageManifest::decode(&mut Cursor::new(&bytes))
            .await
            .unwrap();
        assert!(
            entry
                .extract(&mut Cursor::new(&bytes), &mut tokio::io::sink())
                .await
                .is_err()
        );
    }

    fn frame_package(frame: &[u8], size: u64) -> Vec<u8> {
        let mut bytes = BOOTSTRAP.to_vec();
        bytes.extend_from_slice(frame);
        let offset = bytes.len() as u64;
        bytes.extend(bitcode::encode(&PackageManifest {
            core_config: config(),
            payload_offset: BOOTSTRAP.len() as u64,
            entries: vec![PackageEntry {
                name: "file.bin".into(),
                offset: BOOTSTRAP.len() as u64,
                compressed_size: frame.len() as u64,
                uncompressed_size: size,
            }],
        }));
        footer(&mut bytes, offset);
        bytes
    }

    #[tokio::test]
    async fn reads_previous_v4_frames_and_rejects_truncated_or_concatenated_frames() {
        use std::io::Write;
        let data = b"legacy v4 file contents";
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 0).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(data).unwrap();
        let frame = encoder.finish().unwrap();
        let bytes = frame_package(&frame, data.len() as u64);
        let package = PackageManifest::decode(&mut Cursor::new(&bytes))
            .await
            .unwrap();
        let mut output = Vec::new();
        package.entries[0]
            .extract(&mut Cursor::new(&bytes), &mut output)
            .await
            .unwrap();
        assert_eq!(output, data);
        for end in 1..frame.len() {
            let bytes = frame_package(&frame[..end], data.len() as u64);
            let package = PackageManifest::decode(&mut Cursor::new(&bytes))
                .await
                .unwrap();
            assert!(
                package.entries[0]
                    .extract(&mut Cursor::new(&bytes), &mut tokio::io::sink())
                    .await
                    .is_err()
            );
        }
        for tail in [frame.as_slice(), b"trailing bytes".as_slice()] {
            let mut extended = frame.clone();
            extended.extend_from_slice(tail);
            let bytes = frame_package(&extended, data.len() as u64);
            let package = PackageManifest::decode(&mut Cursor::new(&bytes))
                .await
                .unwrap();
            assert!(
                package.entries[0]
                    .extract(&mut Cursor::new(&bytes), &mut tokio::io::sink())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn rejects_missing_invalid_old_or_oversized_manifest_footers() {
        for bytes in [
            b"short".to_vec(),
            <[u8; Footer::SIZE]>::from(Footer {
                version: Footer::FORMAT_VERSION,
                manifest_offset: 100,
            })
            .to_vec(),
            <[u8; Footer::SIZE]>::from(Footer {
                version: Footer::FORMAT_VERSION - 1,
                manifest_offset: 0,
            })
            .to_vec(),
            <[u8; Footer::SIZE]>::from(Footer {
                version: Footer::FORMAT_VERSION + 1,
                manifest_offset: 0,
            })
            .to_vec(),
        ] {
            assert!(
                PackageManifest::decode(&mut Cursor::new(bytes))
                    .await
                    .is_err()
            );
        }
        let directory = TestDirectory::new();
        let (bytes, _) = encode(&sources(&directory).await).await;
        for case in 0..3 {
            let mut modified = bytes.clone();
            match case {
                0 => modified.pop(),
                1 => {
                    let i = modified.len() - Footer::SIZE;
                    modified[i] ^= 1;
                    None
                }
                _ => {
                    let i = modified.len() - Footer::SIZE;
                    modified.insert(i, 0);
                    None
                }
            };
            assert!(
                PackageManifest::decode(&mut Cursor::new(modified))
                    .await
                    .is_err()
            );
        }
        // A seekable sparse file makes a huge claimed metadata range without allocating it.
        let mut file = File::create(directory.0.join("oversized.bin"))
            .await
            .unwrap();
        file.seek(SeekFrom::Start(MAX_MANIFEST_SIZE + 1))
            .await
            .unwrap();
        file.write_all(&<[u8; Footer::SIZE]>::from(Footer {
            version: Footer::FORMAT_VERSION,
            manifest_offset: 0,
        }))
        .await
        .unwrap();
        file.flush().await.unwrap();
        let mut file = File::open(directory.0.join("oversized.bin")).await.unwrap();
        assert!(
            PackageManifest::decode(&mut file)
                .await
                .unwrap_err()
                .to_string()
                .contains("64 MiB")
        );
    }

    struct VerifyingWriter {
        count: u64,
        largest_write: usize,
    }
    impl AsyncWrite for VerifyingWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            assert!(bytes.len() <= BUFFER_SIZE);
            assert!(bytes.iter().all(|byte| *byte == 42));
            self.largest_write = self.largest_write.max(bytes.len());
            self.count += bytes.len() as u64;
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn streams_a_large_source_and_extraction_through_bounded_buffers() {
        let directory = TestDirectory::new();
        let path = directory.0.join("large.bin");
        let mut source = File::create(&path).await.unwrap();
        let block = vec![42u8; BUFFER_SIZE];
        for _ in 0..1024 {
            source.write_all(&block).await.unwrap();
        }
        source.flush().await.unwrap();
        drop(source);
        let source = SourceManifest {
            core_config: config(),
            entries: vec![SourceEntry::new(&path).unwrap()],
        };
        let package_path = directory.0.join("package.bin");
        let mut writer = tokio::io::BufWriter::new(File::create(&package_path).await.unwrap());
        writer.write_all(BOOTSTRAP).await.unwrap();
        let offset = source
            .encode(&mut writer, BOOTSTRAP.len() as u64)
            .await
            .unwrap();
        writer
            .write_all(&<[u8; Footer::SIZE]>::from(Footer {
                version: Footer::FORMAT_VERSION,
                manifest_offset: offset,
            }))
            .await
            .unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = File::open(package_path).await.unwrap();
        let package = PackageManifest::decode(&mut reader).await.unwrap();
        let mut output = VerifyingWriter {
            count: 0,
            largest_write: 0,
        };
        package.entries[0]
            .extract(&mut reader, &mut output)
            .await
            .unwrap();
        assert_eq!(output.count, 64 * 1024 * 1024);
        assert_eq!(package.entries[0].uncompressed_size, output.count);
        assert_eq!(output.largest_write, BUFFER_SIZE);
    }
}
