//! A page-backed async-aware circular kernel buffer.

use crate::{
    arch::ArchImpl,
    memory::{
        page::ClaimedPage,
        uaccess::{copy_from_user_slice, copy_to_user_slice},
    },
};
use core::{cmp::min, marker::PhantomData, ops::Deref};
use libkernel::{
    error::Result,
    memory::{PAGE_SIZE, address::UA, kbuf::KBufCore},
};
use ringbuf::{storage::Storage, traits::*}; // 引入 traits 以便调用 inner 的 trait 方法

pub struct PageBackedStorage<T>(ClaimedPage, PhantomData<T>);

const USER_COPY_CHUNK_SIZE: usize = 0x100;

unsafe impl<T> Storage for PageBackedStorage<T> {
    type Item = T;

    fn len(&self) -> usize {
        PAGE_SIZE / core::mem::size_of::<T>()
    }

    fn as_mut_ptr(&self) -> *mut core::mem::MaybeUninit<Self::Item> {
        self.0.as_ptr_mut() as *mut _
    }
}

#[derive(Clone)]
pub struct KBuf<T> {
    inner: KBufCore<T, PageBackedStorage<T>, ArchImpl>,
}

impl<T> KBuf<T> {
    pub fn new() -> Result<Self> {
        let pg = ClaimedPage::alloc_zeroed()?;

        Ok(Self {
            inner: KBufCore::new(PageBackedStorage(pg, PhantomData)),
        })
    }

    // === 显式转发同步方法 (修复 cooker.rs 的报错) ===

    /// 尝试推入一个元素，如果满则失败（非阻塞）
    pub fn try_push(&self, item: T) -> core::result::Result<(), T> {
        self.inner.try_push(item)
    }

    /// 尝试推入一个切片，返回实际写入的数量（非阻塞）
    pub fn try_push_slice(&self, elems: &[T]) -> usize
    where
        T: Copy,
    {
        self.inner.try_push_slice(elems)
    }

    /// 尝试弹出一个元素（非阻塞）
    pub fn try_pop(&self) -> Option<T> {
        self.inner.try_pop()
    }

    // === 显式转发异步方法 (修复 tty.rs 的报错) ===

    pub async fn push_slice(&self, elems: &[T]) -> usize
    where
        T: Copy,
    {
        self.inner.push_slice(elems).await
    }

    pub async fn pop_slice(&self, elems: &mut [T]) -> usize
    where
        T: Copy,
    {
        self.inner.pop_slice(elems).await
    }
}

// 保留 Deref 以支持其他基本方法 (如 len, is_empty)
impl<T> Deref for KBuf<T> {
    type Target = KBufCore<T, PageBackedStorage<T>, ArchImpl>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub type KPipe = KBuf<u8>;

impl KPipe {
    /// Copies `count` bytes to the KPipe from a user-space buffer.
    pub async fn copy_from_user(&self, src: UA, count: usize) -> Result<usize> {
        let mut temp_buf = [0u8; USER_COPY_CHUNK_SIZE];
        let chunk_buf = &mut temp_buf[..min(count, USER_COPY_CHUNK_SIZE)];

        copy_from_user_slice(src, chunk_buf).await?;

        Ok(self.push_slice(chunk_buf).await)
    }

    /// Copies `count` bytes from the KPipe to a user-space buffer.
    pub async fn copy_to_user(&self, dst: UA, count: usize) -> Result<usize> {
        let mut temp_buf = [0u8; USER_COPY_CHUNK_SIZE];
        let chunk_buf = &mut temp_buf[..min(count, USER_COPY_CHUNK_SIZE)];

        let bytes_read = self.pop_slice(chunk_buf).await;

        copy_to_user_slice(chunk_buf, dst).await?;

        Ok(bytes_read)
    }

    /// Moves up to `count` bytes from `source` KBuf into `self`.
    pub async fn splice_from(&self, source: &KPipe, count: usize) -> usize {
        self.inner.splice_from(&source.inner, count).await
    }
}