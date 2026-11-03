/* Assembled independently by Clang 19 and executed in both this VM and QEMU.
 * Explicit C instructions exercise all RV64C integer forms; uncompressed
 * checks avoid accidental coverage from assembler relaxation. */
.option rvc
.option norelax
.global _start
.section .text

.macro check reg, value
    .option push
    .option norvc
    li t6, \value
    bne \reg, t6, fail
    .option pop
.endm

_start:
    li a0, 1
    c.nop
    c.li s0, -32
    check s0, -32
    c.addi s0, 31
    check s0, -1
    c.addiw s0, 1
    check s0, 0
    c.lui s0, 0xfffff
    check s0, -4096
    c.slli s0, 32
    check s0, 0xfffff00000000000
    c.srli s0, 63
    check s0, 1
    c.li s0, -1
    c.srai s0, 63
    check s0, -1
    c.andi s0, -32
    check s0, -32
    c.li s1, 7
    c.sub s0, s1
    check s0, -39
    c.xor s0, s1
    check s0, -34
    c.or s0, s1
    check s0, -33
    c.and s0, s1
    check s0, 7
    c.subw s0, s1
    check s0, 0
    c.addw s0, s1
    check s0, 7
    c.mv t0, s0
    c.add t0, s1
    check t0, 14

    li a0, 2
    mv s2, sp
    c.addi16sp sp, -512
    c.addi4spn s0, sp, 1020
    sub t0, s0, sp
    check t0, 1020
    c.addi4spn s0, sp, 4
    sub t0, s0, sp
    check t0, 4
    li s1, -128
    c.swsp s1, 252(sp)
    c.lwsp t0, 252(sp)
    check t0, -128
    c.sdsp s1, 504(sp)
    c.ldsp t0, 504(sp)
    check t0, -128
    c.mv s0, sp
    c.sw s1, 124(s0)
    c.lw a1, 124(s0)
    check a1, -128
    c.sd s1, 248(s0)
    c.ld a1, 248(s0)
    check a1, -128
    c.addi16sp sp, 496
    c.addi sp, 16
    bne sp, s2, fail

    li a0, 3
    c.li s0, 3
1:  c.addi s0, -1
    c.bnez s0, 1b
    c.beqz s0, 2f
    j fail
2:  c.j 3f
    j fail
3:  la t0, 4f
    c.jr t0
    j fail
4:  la t0, 5f
    c.jalr t0
    j 6f
    j fail
5:  c.jr ra
6:
    // Check sign extension of W operations in compressed arithmetic.
    li s0, 0x7fffffff
    c.addiw s0, 1
    check s0, 0xffffffff80000000
    c.li s1, 1
    c.subw s0, s1
    check s0, 0x7fffffff

    .option norvc
    li a0, 4
    li t0, 0x8000000000000000
    li t1, -1
    div t2, t0, t1
    check t2, 0x8000000000000000
    rem t2, t0, t1
    check t2, 0
    mulh t2, t0, t1
    check t2, 0
    mulhsu t2, t0, t1
    check t2, 0x8000000000000000
    divuw t2, t0, zero
    check t2, -1

    li a0, 5
    li t0, 0x8000000080000001
    li t1, 63
    sh1add t2, t0, t1
    check t2, 0x100000041
    sh2add.uw t2, t0, t1
    check t2, 0x200000043
    sh3add.uw t2, t0, zero
    check t2, 0x400000008
    add.uw t2, t0, zero
    check t2, 0x80000001
    slli.uw t2, t0, 32
    check t2, 0x8000000100000000
    andn t2, t0, t1
    check t2, 0x8000000080000000
    orn t2, t0, t1
    check t2, 0xffffffffffffffc1
    xnor t2, t0, zero
    check t2, 0x7fffffff7ffffffe
    clz t2, t0
    check t2, 0
    ctz t2, t0
    check t2, 0
    cpop t2, t0
    check t2, 3
    clzw t2, t0
    check t2, 0
    ctzw t2, t0
    check t2, 0
    cpopw t2, t0
    check t2, 2
    min t2, t0, t1
    bne t2, t0, fail
    minu t2, t0, t1
    check t2, 63
    max t2, t0, t1
    check t2, 63
    maxu t2, t0, t1
    bne t2, t0, fail
    sext.b t2, t0
    check t2, 1
    sext.h t2, t0
    check t2, 1
    zext.h t2, t0
    check t2, 1
    rol t2, t0, t1
    check t2, 0xc000000040000000
    ror t2, t0, t1
    check t2, 0x100000003
    rori t2, t0, 63
    check t2, 0x100000003
    rolw t2, t0, t1
    check t2, 0xffffffffc0000000
    rorw t2, t0, t1
    check t2, 3
    roriw t2, t0, 31
    check t2, 3
    orc.b t2, t0
    check t2, 0xff000000ff0000ff
    rev8 t2, t0
    check t2, 0x0100008000000080
    bclr t2, t0, t1
    check t2, 0x80000001
    bclri t2, t0, 63
    check t2, 0x80000001
    bext t2, t0, t1
    check t2, 1
    bexti t2, t0, 63
    check t2, 1
    binv t2, t0, t1
    check t2, 0x80000001
    binvi t2, t0, 63
    check t2, 0x80000001
    bset t2, zero, t1
    check t2, 0x8000000000000000
    bseti t2, zero, 63
    check t2, 0x8000000000000000

    li a0, 6
    li t0, 0x8000000000000001
    clmul t2, t0, t0
    check t2, 1
    clmulh t2, t0, t0
    check t2, 0x4000000000000000
    clmulr t2, t0, t0
    check t2, 0x8000000000000000
    fence
    fence.i

    li a0, 0
fail:
    li a7, 93
    ecall
    ebreak
