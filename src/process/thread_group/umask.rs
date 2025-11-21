use core::convert::Infallible;

use crate::sched::current_task;

pub fn sys_umask(new_umask: u32) -> core::result::Result<usize, Infallible> {
    let task = current_task();
    let mut umask_guard = task.process.umask.lock_save_irq();

    let old_umask = *umask_guard;

    *umask_guard = new_umask & 0o777;

    Ok(old_umask as _)
}
