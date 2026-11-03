/* Independent TCI-style interpreter: register bytecode, cached translation
 * blocks, helper exits, and guest memory operations. No generated native code.
 * The translator is trusted; all guest-derived addresses are checked here.
 */
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <limits.h>

#define OP(name) name,
enum Opcode {
#include "ops.def"
};
#undef OP

typedef struct {
    uint8_t op, rd, rs1, rs2, immediate, len;
    uint16_t padding;
    uint32_t pc, aux;
    uint64_t imm;
} Insn;

typedef struct {
    uint64_t regs[33];
    uint64_t pc, steps, limit, fault;
    uint8_t *memory;
    const uint8_t *permissions;
    const Insn *const *blocks;
} State;

_Static_assert(sizeof(Insn) == 24, "Rust/C instruction layout");
_Static_assert(offsetof(Insn, imm) == 16, "Rust/C immediate offset");
_Static_assert(offsetof(State, memory) == 296, "Rust/C state layout");
_Static_assert(sizeof(State) == 320, "Rust/C state size");

enum { MISS = 0, SYSCALL = 1, BUDGET = 2, LOAD_FAULT = 3,
       STORE_FAULT = 4, FETCH_FAULT = 5, ILLEGAL = 6, BREAKPOINT = 7 };
enum { MEM_SIZE = 4 * 1024 * 1024, PAGE_SHIFT = 12, READ = 1, WRITE = 2, EXEC = 4 };

static inline uint64_t sx32(uint32_t value) { return (uint64_t)(int64_t)(int32_t)value; }
static inline uint64_t ror64(uint64_t a, unsigned n) {
    n &= 63;
    return (a >> n) | (a << ((-n) & 63));
}
static inline uint32_t ror32(uint32_t a, unsigned n) {
    n &= 31;
    return (a >> n) | (a << ((-n) & 31));
}

static inline int accessible(const uint8_t *p, uint64_t a, unsigned size, unsigned perm) {
    /* Sizes are 1, 2, 4, or 8 bytes. Subtraction avoids integer wraparound.
     * Naturally aligned accesses cannot cross a page; unaligned accesses
     * check both endpoints before any host load/store. */
    return a <= MEM_SIZE - size &&
           (p[a >> PAGE_SHIFT] & perm) &&
           (__builtin_expect((a & (size - 1)) == 0, 1) ||
            (p[(a + size - 1) >> PAGE_SHIFT] & perm));
}

/* Deliberately keep RISC-V divide corner cases away from x86 IDIV traps. */
static uint64_t divs(uint64_t a, uint64_t b) {
    if (!b) return UINT64_MAX;
    if (a == (UINT64_C(1) << 63) && b == UINT64_MAX) return a;
    return (uint64_t)((int64_t)a / (int64_t)b);
}
static uint64_t rems(uint64_t a, uint64_t b) {
    if (!b) return a;
    if (a == (UINT64_C(1) << 63) && b == UINT64_MAX) return 0;
    return (uint64_t)((int64_t)a % (int64_t)b);
}

static __uint128_t carryless(uint64_t a, uint64_t b) {
    __uint128_t result = 0;
    while (b) {
        unsigned bit = (unsigned)__builtin_ctzll(b);
        result ^= (__uint128_t)a << bit;
        b &= b - 1;
    }
    return result;
}

/* Computed-goto bytecode dispatch. The registers, bytecode pointer and
 * instruction count stay local to this
 * loop. Cached successors never make a round trip through Rust. */
int tci_run(State *s) {
#define OP(name) &&op_##name,
    static const void *const dispatch[] = {
#include "ops.def"
    };
#undef OP
    uint64_t *const r = s->regs;
    uint8_t *const mem = s->memory;
    const uint8_t *const perms = s->permissions;
    uint64_t steps = s->steps, target = s->pc;
    const uint64_t limit = s->limit;
    const Insn *ip;
    uint64_t a, b, value, address;

#define RETURN(code, at) do { s->steps = steps; s->pc = (at); return (code); } while (0)
#define DISPATCH() do { \
    if (ip->op != Link && steps == limit) RETURN(BUDGET, ip->pc); \
    a = r[ip->rs1]; b = ip->immediate ? ip->imm : r[ip->rs2]; \
    goto *dispatch[ip->op]; \
} while (0)
#define NEXT() do { ++steps; ++ip; DISPATCH(); } while (0)
#define RESULT(v) do { r[ip->rd] = (v); NEXT(); } while (0)
#define BIN(name, expr) op_##name: RESULT(expr)
#define BRANCH(name, condition) op_##name: \
    target = (condition) ? ip->imm : (uint64_t)ip->pc + ip->len; \
    ++steps; goto lookup

lookup:
    /* Check the budget before touching the next guest instruction. */
    if (steps == limit) RETURN(BUDGET, target);
    if ((target & 1) || target > MEM_SIZE - 2 || !(perms[target >> PAGE_SHIFT] & EXEC)) {
        s->fault = target;
        RETURN(FETCH_FAULT, target);
    }
    ip = s->blocks[target >> 1];
    if (!ip) RETURN(MISS, target);
    DISPATCH();

    op_Nop: NEXT();
    BIN(Mov, b);
    BIN(Add, a + b);
    op_AddLtu:
        if (limit - steps < 2) goto op_Add;
        r[ip->rd] = a + b;
        ++ip;
        /* Read after writeback: the comparison may consume the sum. */
        a = r[ip->rs1]; b = ip->immediate ? ip->imm : r[ip->rs2];
        ++steps;
        RESULT(a < b);
    BIN(Sub, a - b);
    BIN(And, a & b);
    BIN(Or, a | b);
    BIN(Xor, a ^ b);
    BIN(Shl, a << (b & 63));
    BIN(Shr, a >> (b & 63));
    BIN(Sar, (uint64_t)((int64_t)a >> (b & 63)));
    BIN(Lt, (int64_t)a < (int64_t)b);
    BIN(Ltu, a < b);
    BIN(Addw, sx32((uint32_t)(a + b)));
    BIN(Subw, sx32((uint32_t)(a - b)));
    BIN(Shlw, sx32((uint32_t)a << (b & 31)));
    BIN(Shrw, sx32((uint32_t)a >> (b & 31)));
    BIN(Sarw, sx32((uint32_t)((int32_t)a >> (b & 31))));
    BIN(Mul, a * b);
    BIN(Mulh, (uint64_t)(((__int128)(int64_t)a * (int64_t)b) >> 64));
    /* Unsigned multiply then correct the upper half for a's sign. */
    BIN(Mulhsu, (uint64_t)(((__uint128_t)a * b) >> 64) - ((int64_t)a < 0 ? b : 0));
    BIN(Mulhu, (uint64_t)(((__uint128_t)a * b) >> 64));
    op_MulhuMul: {
        if (limit - steps < 2) goto op_Mulhu;
        /* Translation proves matching inputs and no high-result clobber. */
        __uint128_t product = (__uint128_t)a * b;
        r[ip->rd] = (uint64_t)(product >> 64);
        ++ip;
        ++steps;
        RESULT((uint64_t)product);
    }
    BIN(Div, divs(a, b));
    BIN(Divu, b ? a / b : UINT64_MAX);
    BIN(Rem, rems(a, b));
    BIN(Remu, b ? a % b : a);
    BIN(Mulw, sx32((uint32_t)(a * b)));
    BIN(Divw, sx32((uint32_t)divs(sx32((uint32_t)a), sx32((uint32_t)b))));
    BIN(Divuw, sx32((uint32_t)b ? (uint32_t)a / (uint32_t)b : UINT32_MAX));
    BIN(Remw, sx32((uint32_t)rems(sx32((uint32_t)a), sx32((uint32_t)b))));
    BIN(Remuw, sx32((uint32_t)b ? (uint32_t)a % (uint32_t)b : (uint32_t)a));

#define LOAD(name, type, convert) op_##name: { \
    address = a + ip->imm; \
    if (!accessible(perms, address, sizeof(type), READ)) goto load_fault; \
    type loaded; memcpy(&loaded, mem + address, sizeof(loaded)); \
    RESULT(convert); \
}
    LOAD(Lb, int8_t, (uint64_t)(int64_t)loaded);
    LOAD(Lh, int16_t, (uint64_t)(int64_t)loaded);
    LOAD(Lw, int32_t, (uint64_t)(int64_t)loaded);
    LOAD(Ld, uint64_t, loaded);
    LOAD(Lbu, uint8_t, loaded);
    LOAD(Lhu, uint16_t, loaded);
    LOAD(Lwu, uint32_t, loaded);

    op_LdLd: {
        if (limit - steps < 2) goto op_Ld;
        address = a + ip->imm;
        if (!accessible(perms, address, sizeof(uint64_t), READ)) goto load_fault;
        uint64_t loaded; memcpy(&loaded, mem + address, sizeof(loaded));
        r[ip->rd] = loaded;
        ++steps; ++ip;
        /* The first load can change the second load's base. Advance the
         * retirement count and PC before checking the second access. */
        a = r[ip->rs1];
        goto op_Ld;
    }

#define STORE(name, type) op_##name: { \
    address = a + ip->imm; \
    if (!accessible(perms, address, sizeof(type), WRITE)) goto store_fault; \
    type stored = (type)r[ip->rs2]; \
    memcpy(mem + address, &stored, sizeof(stored)); NEXT(); \
}
    STORE(Sb, uint8_t);
    STORE(Sh, uint16_t);
    STORE(Sw, uint32_t);
    STORE(Sd, uint64_t);

    op_SdSd: {
        if (limit - steps < 2) goto op_Sd;
        address = a + ip->imm;
        if (!accessible(perms, address, sizeof(uint64_t), WRITE)) goto store_fault;
        uint64_t stored = r[ip->rs2];
        memcpy(mem + address, &stored, sizeof(stored));
        ++steps; ++ip;
        a = r[ip->rs1];
        goto op_Sd;
    }

    BRANCH(Beq, a == b);
    BRANCH(Bne, a != b);
    BRANCH(Blt, (int64_t)a < (int64_t)b);
    BRANCH(Bge, (int64_t)a >= (int64_t)b);
    BRANCH(Bltu, a < b);
    BRANCH(Bgeu, a >= b);
    op_Jump:
        target = ip->imm;
        r[ip->rd] = (uint64_t)ip->pc + ip->len;
        ++steps; goto lookup;
    op_JumpReg:
        target = (a + ip->imm) & ~UINT64_C(1);
        r[ip->rd] = (uint64_t)ip->pc + ip->len;
        ++steps; goto lookup;
    op_Ecall:
        ++steps; RETURN(SYSCALL, ip->pc);
    op_Break:
        RETURN(BREAKPOINT, ip->pc);

    BIN(Andn, a & ~b);
    BIN(Orn, a | ~b);
    BIN(Xnor, ~(a ^ b));
    BIN(Clz, a ? (unsigned)__builtin_clzll(a) : 64);
    BIN(Ctz, a ? (unsigned)__builtin_ctzll(a) : 64);
    BIN(Cpop, (unsigned)__builtin_popcountll(a));
    BIN(Clzw, (uint32_t)a ? (unsigned)__builtin_clz((uint32_t)a) : 32);
    BIN(Ctzw, (uint32_t)a ? (unsigned)__builtin_ctz((uint32_t)a) : 32);
    BIN(Cpopw, (unsigned)__builtin_popcount((uint32_t)a));
    BIN(Min, (int64_t)a < (int64_t)b ? a : b);
    BIN(Max, (int64_t)a > (int64_t)b ? a : b);
    BIN(Minu, a < b ? a : b);
    BIN(Maxu, a > b ? a : b);
    BIN(Sextb, (uint64_t)(int64_t)(int8_t)a);
    BIN(Sexth, (uint64_t)(int64_t)(int16_t)a);
    BIN(Zexth, (uint16_t)a);
    BIN(Rol, ror64(a, (unsigned)(0 - b)));
    BIN(Ror, ror64(a, (unsigned)b));
    BIN(Rolw, sx32(ror32((uint32_t)a, (unsigned)(0 - b))));
    BIN(Rorw, sx32(ror32((uint32_t)a, (unsigned)b)));
    op_Orcb:
        value = 0;
        for (unsigned i = 0; i < 64; i += 8) {
            if ((a >> i) & 255) value |= UINT64_C(255) << i;
        }
        RESULT(value);
    BIN(Rev8, __builtin_bswap64(a));
    BIN(Shadd, (a << ip->aux) + b);
    BIN(Shadduw, ((uint64_t)(uint32_t)a << ip->aux) + b);
    BIN(Adduw, (uint64_t)(uint32_t)a + b);
    BIN(Shluw, (uint64_t)(uint32_t)a << (b & 63));
    BIN(Bclr, a & ~(UINT64_C(1) << (b & 63)));
    BIN(Bext, (a >> (b & 63)) & 1);
    BIN(Binv, a ^ (UINT64_C(1) << (b & 63)));
    BIN(Bset, a | (UINT64_C(1) << (b & 63)));
    BIN(Clmul, (uint64_t)carryless(a, b));
    BIN(Clmulh, (uint64_t)(carryless(a, b) >> 64));
    BIN(Clmulr, (uint64_t)(carryless(a, b) >> 63));
    op_Link:
        target = ip->imm; goto lookup;
    op_Trap:
        s->fault = ip->imm; RETURN((int)ip->aux, ip->pc);

load_fault:
    s->fault = address; RETURN(LOAD_FAULT, ip->pc);
store_fault:
    s->fault = address; RETURN(STORE_FAULT, ip->pc);
}
