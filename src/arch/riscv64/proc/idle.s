.section .text
.global __idle_start
.global __idle_end

.balign 4
__idle_start:
    // 循环执行 wfi
1:
    wfi
    j 1b

__idle_end: