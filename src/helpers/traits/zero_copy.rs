use dashmap::DashMap;
use memmap2::Mmap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};
use crate::dev_print;

use crate::SendableError;

#[derive(Debug)]
pub struct ZeroCopyFile {
    mmap: Mmap,
    _file: File,
    last_accessed: SystemTime,
    file_size: usize,
}

impl ZeroCopyFile {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, SendableError> {
        let file = File::open(path.as_ref())?;
        let metadata = file.metadata()?;
        let file_size = metadata.len() as usize;
        
        // 빈 파일 처리
        if file_size == 0 {
            return Err("Cannot memory map empty file".into());
        }

        // SAFETY: 파일이 외부에서 수정되지 않는다고 가정
        let mmap = unsafe { Mmap::map(&file)? };

        Ok(Self {
            mmap,
            _file: file,
            last_accessed: SystemTime::now(),
            file_size,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap
    }

    pub fn as_str(&self) -> Result<&str, SendableError> {
        Ok(std::str::from_utf8(&self.mmap)?)
    }

    pub fn len(&self) -> usize {
        self.file_size
    }

    pub fn is_empty(&self) -> bool {
        self.file_size == 0
    }

    pub fn update_access_time(&mut self) {
        self.last_accessed = SystemTime::now();
    }

    pub fn last_accessed(&self) -> SystemTime {
        self.last_accessed
    }

    /// JSON 파싱 지원 (제로카피)
    pub fn parse_json<T>(&self) -> Result<T, SendableError>
    where
        T: for<'de> serde::de::Deserialize<'de>,
    {
        let json_str = self.as_str()?;
        Ok(serde_json::from_str(json_str)?)
    }

    /// 특정 범위의 바이트 슬라이스 (제로카피)
    pub fn slice(&self, start: usize, end: usize) -> Result<&[u8], SendableError> {
        if end > self.mmap.len() || start > end {
            return Err(format!(
                "Index out of bounds: start={}, end={}, len={}",
                start, end, self.mmap.len()
            ).into());
        }
        Ok(&self.mmap[start..end])
    }

    /// 라인별 순회 이터레이터 (제로카피)
    pub fn lines(&self) -> LineIterator<'_> {
        LineIterator {
            data: &self.mmap,
            pos: 0,
        }
    }

    /// 특정 패턴 찾기
    pub fn find(&self, pattern: &[u8]) -> Option<usize> {
        self.mmap
            .windows(pattern.len())
            .position(|window| window == pattern)
    }
}

pub struct LineIterator<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for LineIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.data.len() {
            return None;
        }

        let start = self.pos;

        // \n 또는 \r\n 찾기
        while self.pos < self.data.len() {
            if self.data[self.pos] == b'\n' {
                let end = self.pos;
                self.pos += 1;

                // \r\n 처리
                let line_end = if end > start && self.data[end - 1] == b'\r' {
                    end - 1
                } else {
                    end
                };

                return Some(&self.data[start..line_end]);
            }
            self.pos += 1;
        }

        // 마지막 줄 (개행 문자 없음)
        if start < self.data.len() {
            Some(&self.data[start..])
        } else {
            None
        }
    }
}

/// 캐시된 파일 데이터 (메모리에 복사본 저장)
#[derive(Debug)]
pub struct CachedFileData {
    data: Vec<u8>,
    last_accessed: SystemTime,
    file_path: PathBuf,
    original_size: usize,
}

impl CachedFileData {
    pub fn new(data: Vec<u8>, file_path: PathBuf) -> Self {
        let original_size = data.len();
        Self {
            data,
            last_accessed: SystemTime::now(),
            file_path,
            original_size,
        }
    }

    pub fn get_info(&self) -> (PathBuf, usize) {
        (self.file_path.clone(), self.original_size)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn as_str(&self) -> Result<&str, SendableError> {
        Ok(std::str::from_utf8(&self.data)?)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn update_access_time(&mut self) {
        self.last_accessed = SystemTime::now();
    }

    pub fn last_accessed(&self) -> SystemTime {
        self.last_accessed
    }

    /// JSON 파싱 (캐시된 데이터에서)
    pub fn parse_json<T>(&self) -> Result<T, SendableError>
    where
        T: for<'de> serde::de::Deserialize<'de>,
    {
        let json_str = self.as_str()?;
        Ok(serde_json::from_str(json_str)?)
    }
}

/// 캐시 설정 구조체
#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub max_cache_files: usize,
    pub max_cache_file_size: usize,
    pub cache_duration: Duration,
    pub total_cache_size_limit: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_cache_files: 50,                    // 최대 50개 파일
            max_cache_file_size: 1024 * 1024,      // 1MB 이하 파일만 메모리 캐시
            total_cache_size_limit: 50 * 1024 * 1024, // 총 50MB 캐시 제한
            cache_duration: Duration::from_secs(300),  // 5분 캐시 유지
        }
    }
}

/// 하이브리드 파일 캐시 관리자
/// - 작은 파일: 메모리에 완전히 로드하여 캐시
/// - 큰 파일: 필요할 때마다 memmap2 사용 (캐시 안함)
pub struct ZeroCopyCache {
    // 작은 파일들의 메모리 캐시
    memory_cache: Arc<DashMap<PathBuf, CachedFileData>>,
    max_cache_files: usize,
    max_cache_file_size: usize,  // 이 크기 이하만 메모리 캐시
    cache_duration: Duration,
    total_cache_size_limit: usize,
}

// 전역 캐시 인스턴스
static GLOBAL_CACHE: OnceLock<ZeroCopyCache> = OnceLock::new();

impl ZeroCopyCache {
    pub fn new(
        max_cache_files: usize,
        max_cache_file_size: usize,
        total_cache_size_limit: usize,
        cache_duration: Duration
    ) -> Self {
        Self {
            memory_cache: Arc::new(DashMap::new()),
            max_cache_files,
            max_cache_file_size,
            total_cache_size_limit,
            cache_duration,
        }
    }

    /// 설정으로부터 캐시 생성
    pub fn from_config(config: CacheConfig) -> Self {
        Self::new(
            config.max_cache_files,
            config.max_cache_file_size,
            config.total_cache_size_limit,
            config.cache_duration,
        )
    }

    /// 기본 설정으로 캐시 생성
    pub fn default() -> Self {
        Self::from_config(CacheConfig::default())
    }

    /// 전역 캐시 초기화 (한 번만 호출)
    pub fn init_global(config: Option<CacheConfig>) -> Result<(), &'static str> {
        let cache_config = config.unwrap_or_default();
        let cache = Self::from_config(cache_config);

        GLOBAL_CACHE.set(cache)
            .map_err(|_| "Global cache already initialized")
    }

    /// 전역 캐시 인스턴스 반환
    pub fn global() -> &'static ZeroCopyCache {
        GLOBAL_CACHE.get_or_init(|| Self::default())
    }

    /// 파일을 로드 (캐시 사용 또는 직접 로드)
    pub fn load_file<P: AsRef<Path>>(&self, path: P) -> Result<FileLoadResult, SendableError> {

        let path_buf = path.as_ref().to_path_buf();
        
        // 파일 크기 확인
        let metadata = std::fs::metadata(&path_buf)?;
        let file_size = metadata.len() as usize;
        
        // 작은 파일: 메모리 캐시 사용
        if file_size <= self.max_cache_file_size {
            return self.load_from_memory_cache(path_buf, file_size);
        }
        
        // 큰 파일: 직접 memmap2 사용 (캐시 안함)
        dev_print!("Large file detected ({}MB), using direct memmap", file_size / (1024 * 1024));
        let zero_copy_file = ZeroCopyFile::new(&path_buf)?;
        Ok(FileLoadResult::DirectMemoryMap(zero_copy_file))
    }

    /// 메모리 캐시에서 파일 로드
    fn load_from_memory_cache(&self, path_buf: PathBuf, file_size: usize) -> Result<FileLoadResult, SendableError> {
        // 캐시에서 먼저 찾기
        {
            if let Some(mut cached_data) = self.memory_cache.get_mut(&path_buf) {
                cached_data.update_access_time();
                dev_print!("File loaded from memory cache: {:?} ({}KB)", path_buf, file_size / 1024);
                return Ok(FileLoadResult::MemoryCache(cached_data.data.clone()));
            }
        }

        // 캐시에 없으면 파일을 읽어서 메모리에 저장
        dev_print!("Loading file to memory cache: {:?} ({}KB)", path_buf, file_size / 1024);
        let file_data = std::fs::read(&path_buf)?;
        let cached_data = CachedFileData::new(file_data.clone(), path_buf.clone());

        // 캐시에 추가
        {
            // 캐시 용량 관리
            self.ensure_cache_capacity(self.memory_cache.clone(), file_size);

            self.memory_cache.insert(path_buf, cached_data);
        }

        Ok(FileLoadResult::MemoryCache(file_data))
    }

    /// 캐시 용량 확보
    fn ensure_cache_capacity(&self, cache:  Arc<DashMap<PathBuf, CachedFileData>>, new_file_size: usize) {
        // 파일 개수 제한 확인
        while cache.len() >= self.max_cache_files {
            self.evict_oldest_from_cache(&cache);
        }

        // 총 크기 제한 확인
        let current_total_size: usize = cache.iter().map(|f| f.original_size).sum();
        let mut remaining_size = current_total_size + new_file_size;

        while remaining_size > self.total_cache_size_limit && !cache.is_empty() {
            if let Some(evicted_size) = self.evict_oldest_from_cache(&cache) {
                remaining_size -= evicted_size;
            } else {
                break;
            }
        }
    }

    /// 가장 오래된 항목 제거
    fn evict_oldest_from_cache(&self, cache: &Arc<DashMap<PathBuf, CachedFileData>>) -> Option<usize> {
        if cache.is_empty() {
            return None;
        }

        let oldest_entry = cache
            .iter()
            .min_by_key(|file| file.last_accessed())
            .map(|file| (file.key().clone(), file.len()));

        if let Some((oldest_path, size)) = oldest_entry {
            cache.remove(&oldest_path);
            dev_print!("Evicted file from memory cache: {:?} ({}KB)", oldest_path, size / 1024);
            return Some(size);
        }

        None
    }

    /// 만료된 캐시 항목 정리
    pub fn cleanup_expired(&self) {
        let now = SystemTime::now();
        
        let expired_keys: Vec<PathBuf> = self.memory_cache
            .iter()
            .filter_map( |file| {
                if now.duration_since(file.last_accessed()).unwrap_or_default() > self.cache_duration {
                    Some(file.key().clone())
                } else {
                    None
                }
            })
            .collect();

        for key in expired_keys {
            if let Some((_path, removed)) = self.memory_cache.remove(&key) {
                dev_print!("Removed expired file from cache: {:?} ({}KB)", key, removed.original_size / 1024);
            }
        }
    }

    /// 캐시 통계
    pub fn stats(&self) -> CacheStats {
        let total_size: usize = self.memory_cache.iter().map(|f| f.original_size).sum();
        
        CacheStats {
            file_count: self.memory_cache.len(),
            total_size,
            max_cache_size: self.max_cache_files,
            max_file_size: self.max_cache_file_size,
            total_cache_size_limit: self.total_cache_size_limit,
        }
    }

    /// 캐시 강제 정리
    pub fn clear_cache(&self) {
        let count = self.memory_cache.len();
        let total_size: usize = self.memory_cache.iter().map(|f| f.original_size).sum();
        self.memory_cache.clear();
        dev_print!("Cleared memory cache: {} files, {}MB", count, total_size / (1024 * 1024));
    }
}

/// 파일 로드 결과
pub enum FileLoadResult {
    /// 메모리 캐시에서 로드된 데이터 (소유권 있는 복사본)
    MemoryCache(Vec<u8>),
    /// 직접 메모리 매핑된 파일 (큰 파일용)
    DirectMemoryMap(ZeroCopyFile),
}

impl FileLoadResult {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            FileLoadResult::MemoryCache(data) => data,
            FileLoadResult::DirectMemoryMap(mmap_file) => mmap_file.as_bytes(),
        }
    }

    pub fn as_str(&self) -> Result<&str, SendableError> {
        match self {
            FileLoadResult::MemoryCache(data) => Ok(std::str::from_utf8(data)?),
            FileLoadResult::DirectMemoryMap(mmap_file) => mmap_file.as_str(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            FileLoadResult::MemoryCache(data) => data.len(),
            FileLoadResult::DirectMemoryMap(mmap_file) => mmap_file.len(),
        }
    }

    pub fn parse_json<T>(&self) -> Result<T, SendableError>
    where
        T: for<'de> serde::de::Deserialize<'de>,
    {
        match self {
            FileLoadResult::MemoryCache(data) => {
                let json_str = std::str::from_utf8(data)?;
                Ok(serde_json::from_str(json_str)?)
            }
            FileLoadResult::DirectMemoryMap(mmap_file) => mmap_file.parse_json(),
        }
    }

    pub fn is_memory_cached(&self) -> bool {
        matches!(self, FileLoadResult::MemoryCache(_))
    }

    pub fn is_memory_mapped(&self) -> bool {
        matches!(self, FileLoadResult::DirectMemoryMap(_))
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub file_count: usize,
    pub total_size: usize,
    pub max_cache_size: usize,
    pub max_file_size: usize,
    pub total_cache_size_limit: usize,
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, 
            "Memory Cache Stats: {} files, {:.2}/{:.2} MB used, max {} files, max {:.2} MB per file",
            self.file_count,
            self.total_size as f64 / 1_048_576.0,
            self.total_cache_size_limit as f64 / 1_048_576.0,
            self.max_cache_size,
            self.max_file_size as f64 / 1_048_576.0
        )
    }
}

/// JSON 파일을 제로카피로 파싱하는 헬퍼 함수
pub fn parse_json_file<T, P>(path: P) -> Result<T, SendableError>
where
    T: for<'de> serde::de::Deserialize<'de>,
    P: AsRef<Path>,
{
    // 파일 크기에 따라 적절한 방법 선택
    let metadata = std::fs::metadata(&path)?;
    let file_size = metadata.len() as usize;
    
    if file_size <= 1024 * 1024 { // 1MB 이하는 일반 읽기
        dev_print!("Small JSON file, using standard read: {}KB", file_size / 1024);
        let data = std::fs::read(&path)?;
        let json_str = std::str::from_utf8(&data)?;
        Ok(serde_json::from_str(json_str)?)
    } else { // 큰 파일은 memmap2 사용
        dev_print!("Large JSON file, using zero-copy mmap: {}MB", file_size / (1024 * 1024));
        let zero_copy_file = ZeroCopyFile::new(path)?;
        zero_copy_file.parse_json()
    }
}

/// 캐시를 사용한 JSON 파일 파싱
pub fn parse_json_file_cached<T, P>(path: P, cache: &mut ZeroCopyCache) -> Result<T, SendableError>
where
    T: for<'de> serde::de::Deserialize<'de>,
    P: AsRef<Path>,
{
    let file_result = cache.load_file(path)?;
    file_result.parse_json()
}