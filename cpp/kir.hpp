// kir.hpp - KIR parser, machine file reader, interpreter and timing model.
// A C++ port of the parts of the starter kit's kirlib.py that kopt needs.
#pragma once
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <list>
#include <map>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

typedef long long ll;
typedef unsigned long long ull;
typedef uint32_t u32;

static const ll MASK = 0xFFFFFFFFLL;
static const ll SIGNB = 0x80000000LL;
static const int MEM_WORDS = 4096;
static const int SCRATCH_BASE = 3072;
static const int MAX_INSTRS = 500000;
static const int MAX_PARAMS = 8;

struct KirError : std::runtime_error {
    explicit KirError(const std::string& m) : std::runtime_error(m) {}
};

enum Opc {
    O_ADD, O_SUB, O_MUL, O_AND, O_OR, O_XOR, O_SHL, O_SHR, O_LT, O_EQ, O_DIV, O_MOD,
    O_CONST, O_MOV, O_SELECT, O_MAC, O_ADDSH, O_LOAD, O_STORE, O_VLOAD, O_VSTORE,
    O_TRAP, O_JMP, O_BR, O_RET, O_PHI, O_NOPS
};
static const char* const OPNAME[] = {
    "add", "sub", "mul", "and", "or", "xor", "shl", "shr", "lt", "eq", "div", "mod",
    "const", "mov", "select", "mac", "addsh", "load", "store", "vload", "vstore",
    "trap", "jmp", "br", "ret", "phi"};
// d = destination, s = source, i = literal, k = shift literal, L = label
static const char* const OPSIG[] = {
    "dss", "dss", "dss", "dss", "dss", "dss", "dss", "dss", "dss", "dss", "dss", "dss",
    "di", "ds", "dsss", "dsss", "dssk", "ds", "ss", "ds", "ss",
    "s", "L", "sLL", "s", ""};

inline bool is_term(Opc o) { return o == O_JMP || o == O_BR || o == O_RET || o == O_TRAP; }
inline bool is_memop(Opc o) { return o == O_LOAD || o == O_STORE || o == O_VLOAD || o == O_VSTORE; }
inline bool is_binop(Opc o) { return o <= O_MOD; }
inline bool is_comm(Opc o) { return o == O_ADD || o == O_MUL || o == O_AND || o == O_OR || o == O_XOR || o == O_EQ; }
// ops with a destination and no side effect at all
inline bool is_pure(Opc o) { return o <= O_EQ || o == O_SELECT || o == O_MAC || o == O_ADDSH; }

inline ll LIT(ll v) { return ~(v & MASK); }

// --------------------------------------------------------------------------
// Parsed program
// --------------------------------------------------------------------------
struct PInstr {
    Opc op;
    ll d = -1;
    std::vector<ll> a;          // operands: >= 0 register, < 0 literal ~x
    std::string t[2];
    int nt = 0;
};
struct PBlock {
    std::string label;
    std::vector<PInstr> ins;
};
struct Program {
    std::string name;
    std::vector<ll> params;
    std::vector<PBlock> blocks;
    int ninstr = 0;
};

static inline bool ws(char c) { return c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == '\v' || c == '\f'; }
static inline std::string strip(const std::string& s) {
    size_t a = 0, b = s.size();
    while (a < b && ws(s[a])) a++;
    while (b > a && ws(s[b - 1])) b--;
    return s.substr(a, b - a);
}
static inline bool is_name(const std::string& s) {
    if (s.empty()) return false;
    if (!(isalpha((unsigned char)s[0]) || s[0] == '_')) return false;
    for (char c : s)
        if (!(isalnum((unsigned char)c) || c == '_')) return false;
    for (char c : s)
        if ((unsigned char)c >= 128) return false;
    return true;
}
static inline bool is_reg(const std::string& s) {
    if (s.size() < 2 || s[0] != 'v') return false;
    if (s.size() > 7) return false;
    if (s[1] == '0') return s.size() == 2;
    for (size_t i = 1; i < s.size(); i++)
        if (s[i] < '0' || s[i] > '9') return false;
    return true;
}
static inline bool parse_lit(const std::string& tok, ll& out) {
    size_t i = 0;
    bool neg = false;
    if (i < tok.size() && tok[i] == '-') { neg = true; i++; }
    if (i >= tok.size()) return false;
    ull n = 0;
    if (tok.size() - i > 2 && tok[i] == '0' && tok[i + 1] == 'x') {
        i += 2;
        if (i >= tok.size()) return false;
        for (; i < tok.size(); i++) {
            char c = tok[i];
            int dgt;
            if (c >= '0' && c <= '9') dgt = c - '0';
            else if (c >= 'a' && c <= 'f') dgt = c - 'a' + 10;
            else if (c >= 'A' && c <= 'F') dgt = c - 'A' + 10;
            else return false;
            n = n * 16 + dgt;
            if (n > (1ULL << 40)) throw KirError("literal does not fit in 32 bits");
        }
    } else {
        for (; i < tok.size(); i++) {
            char c = tok[i];
            if (c < '0' || c > '9') return false;
            n = n * 10 + (c - '0');
            if (n > (1ULL << 40)) throw KirError("literal does not fit in 32 bits");
        }
    }
    ll v = neg ? -(ll)n : (ll)n;
    if (v < -(1LL << 31) || v > MASK) throw KirError("literal does not fit in 32 bits");
    out = v;
    return true;
}

static inline std::vector<std::string> split_lines(const std::string& text) {
    std::vector<std::string> out;
    std::string cur;
    size_t n = text.size();
    for (size_t i = 0; i < n; i++) {
        char c = text[i];
        if (c == '\n' || c == '\r' || c == '\v' || c == '\f') {
            if (c == '\r' && i + 1 < n && text[i + 1] == '\n') i++;
            out.push_back(cur);
            cur.clear();
        } else {
            cur.push_back(c);
        }
    }
    if (!cur.empty()) out.push_back(cur);
    return out;
}

static inline Program parse_program(const std::string& text) {
    Program prog;
    bool seen_func = false;
    std::set<std::string> labels;
    std::map<std::string, Opc> opid;
    for (int i = 0; i < O_PHI; i++) opid[OPNAME[i]] = (Opc)i;
    for (const std::string& raw : split_lines(text)) {
        std::string line = raw;
        size_t sc = line.find(';');
        if (sc != std::string::npos) line = line.substr(0, sc);
        line = strip(line);
        if (line.empty()) continue;
        // func header
        if (line.size() > 5 && line.compare(0, 4, "func") == 0 && ws(line[4]) && line.back() == ')') {
            size_t i = 4;
            while (i < line.size() && ws(line[i])) i++;
            size_t j = i;
            while (j < line.size() && (isalnum((unsigned char)line[j]) || line[j] == '_')) j++;
            std::string name = line.substr(i, j - i);
            size_t k = j;
            while (k < line.size() && ws(line[k])) k++;
            if (is_name(name) && k < line.size() && line[k] == '(') {
                if (seen_func) throw KirError("only one func header is allowed");
                if (!prog.blocks.empty()) throw KirError("func header must come before the first label");
                seen_func = true;
                prog.name = name;
                std::string plist = strip(line.substr(k + 1, line.size() - k - 2));
                if (!plist.empty()) {
                    std::stringstream ss(plist);
                    std::string tok;
                    size_t pos = 0;
                    while (true) {
                        size_t c = plist.find(',', pos);
                        tok = strip(plist.substr(pos, c == std::string::npos ? std::string::npos : c - pos));
                        if (!is_reg(tok)) throw KirError("parameter is not a register");
                        ll idx = atoll(tok.c_str() + 1);
                        for (ll p : prog.params)
                            if (p == idx) throw KirError("parameter listed twice");
                        prog.params.push_back(idx);
                        if (c == std::string::npos) break;
                        pos = c + 1;
                    }
                }
                if ((int)prog.params.size() > MAX_PARAMS) throw KirError("too many parameters");
                continue;
            }
        }
        if (!seen_func) throw KirError("expected func header first");
        if (line.back() == ':' && is_name(line.substr(0, line.size() - 1))) {
            std::string name = line.substr(0, line.size() - 1);
            if (is_reg(name)) throw KirError("label looks like a register");
            if (labels.count(name)) throw KirError("label defined twice");
            labels.insert(name);
            prog.blocks.push_back(PBlock());
            prog.blocks.back().label = name;
            continue;
        }
        size_t sp = 0;
        while (sp < line.size() && !ws(line[sp])) sp++;
        std::string op = line.substr(0, sp);
        auto it = opid.find(op);
        if (it == opid.end()) throw KirError("unknown instruction '" + op + "'");
        if (prog.blocks.empty()) throw KirError("instruction outside of a block");
        std::string rest = strip(line.substr(sp));
        std::vector<std::string> toks;
        if (!rest.empty()) {
            size_t pos = 0;
            while (true) {
                size_t c = rest.find(',', pos);
                toks.push_back(strip(rest.substr(pos, c == std::string::npos ? std::string::npos : c - pos)));
                if (c == std::string::npos) break;
                pos = c + 1;
            }
        }
        const char* sig = OPSIG[it->second];
        if (toks.size() != strlen(sig)) throw KirError("wrong number of operands for " + op);
        PInstr pi;
        pi.op = it->second;
        for (size_t i = 0; i < toks.size(); i++) {
            const std::string& tok = toks[i];
            if (tok.empty()) throw KirError("empty operand");
            char kind = sig[i];
            ll v;
            if (kind == 'd') {
                if (!is_reg(tok)) throw KirError("expected a register");
                pi.d = atoll(tok.c_str() + 1);
            } else if (kind == 's') {
                if (is_reg(tok)) pi.a.push_back(atoll(tok.c_str() + 1));
                else if (parse_lit(tok, v)) pi.a.push_back(LIT(v));
                else throw KirError("expected a register or literal");
            } else if (kind == 'i') {
                if (!parse_lit(tok, v)) throw KirError("bad integer literal");
                pi.a.push_back(LIT(v));
            } else if (kind == 'k') {
                if (is_reg(tok) || !parse_lit(tok, v)) throw KirError("bad shift literal");
                if (v < 0 || v > 31) throw KirError("shift amount of addsh must be 0..31");
                pi.a.push_back(LIT(v));
            } else {
                if (!is_name(tok) || is_reg(tok)) throw KirError("expected a label name");
                pi.t[pi.nt++] = tok;
            }
        }
        prog.blocks.back().ins.push_back(pi);
    }
    if (!seen_func) throw KirError("missing func header");
    if (prog.blocks.empty()) throw KirError("the function has no blocks");
    int total = 0;
    for (auto& b : prog.blocks) {
        if (b.ins.empty()) throw KirError("block '" + b.label + "' is empty");
        for (size_t i = 0; i + 1 < b.ins.size(); i++)
            if (is_term(b.ins[i].op)) throw KirError("terminator before the end of a block");
        if (!is_term(b.ins.back().op)) throw KirError("block must end with jmp, br, ret or trap");
        total += (int)b.ins.size();
    }
    if (total > MAX_INSTRS) throw KirError("program too large");
    for (auto& b : prog.blocks)
        for (auto& pi : b.ins)
            for (int i = 0; i < pi.nt; i++)
                if (!labels.count(pi.t[i])) throw KirError("jump to undefined label '" + pi.t[i] + "'");
    prog.ninstr = total;
    return prog;
}

// Read a text file: UTF-8, UTF-8 with BOM, or UTF-16 with BOM.
static inline std::string read_text(const std::string& path) {
    std::ifstream f(path.c_str(), std::ios::binary);
    if (!f) throw KirError("cannot read " + path);
    std::stringstream ss;
    ss << f.rdbuf();
    std::string d = ss.str();
    if (d.size() >= 3 && (unsigned char)d[0] == 0xEF && (unsigned char)d[1] == 0xBB && (unsigned char)d[2] == 0xBF)
        return d.substr(3);
    if (d.size() >= 2 && (((unsigned char)d[0] == 0xFF && (unsigned char)d[1] == 0xFE) ||
                          ((unsigned char)d[0] == 0xFE && (unsigned char)d[1] == 0xFF))) {
        bool le = (unsigned char)d[0] == 0xFF;
        std::string out;
        for (size_t i = 2; i + 1 < d.size(); i += 2) {
            unsigned c = le ? ((unsigned char)d[i] | ((unsigned char)d[i + 1] << 8))
                            : ((unsigned char)d[i + 1] | ((unsigned char)d[i] << 8));
            out.push_back(c < 128 ? (char)c : '?');
        }
        return out;
    }
    return d;
}

// --------------------------------------------------------------------------
// Machine
// --------------------------------------------------------------------------
struct Machine {
    ll issue_width, mem_ports, registers, reg_spill_penalty, taken_branch_penalty, icache_size,
        icache_penalty, lat_alu, lat_mul, lat_div, lat_load, lat_mac, lat_addsh, fused_mac, fused_addsh;
    ll lat(Opc o) const {
        switch (o) {
            case O_MUL: return lat_mul;
            case O_DIV: case O_MOD: return lat_div;
            case O_LOAD: case O_VLOAD: return lat_load;
            case O_MAC: return lat_mac;
            case O_ADDSH: return lat_addsh;
            default: return lat_alu;
        }
    }
};

static inline Machine load_machine(const std::string& path) {
    static const char* const keys[] = {"issue_width", "mem_ports", "registers", "reg_spill_penalty",
        "taken_branch_penalty", "icache_size", "icache_penalty", "lat_alu", "lat_mul", "lat_div",
        "lat_load", "lat_mac", "lat_addsh", "fused_mac", "fused_addsh"};
    static const ll mins[] = {1, 1, 4, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 0};
    std::map<std::string, ll> mc;
    for (const std::string& raw : split_lines(read_text(path))) {
        std::string line = raw;
        size_t h = line.find('#');
        if (h != std::string::npos) line = line.substr(0, h);
        line = strip(line);
        if (line.empty()) continue;
        size_t e = line.find('=');
        if (e == std::string::npos) throw KirError("machine: expected key = value");
        std::string key = strip(line.substr(0, e)), val = strip(line.substr(e + 1));
        int ki = -1;
        for (int i = 0; i < 15; i++)
            if (key == keys[i]) ki = i;
        if (ki < 0) throw KirError("machine: unknown key " + key);
        if (mc.count(key)) throw KirError("machine: key given twice");
        if (val.empty() || val.size() > 9) throw KirError("machine: bad value");
        for (char c : val)
            if (c < '0' || c > '9') throw KirError("machine: bad value");
        ll n = atoll(val.c_str());
        if (n < mins[ki] || n > 1000000) throw KirError("machine: value out of range");
        if (key.compare(0, 6, "fused_") == 0 && n > 1) throw KirError("machine: fused flag must be 0 or 1");
        mc[key] = n;
    }
    for (int i = 0; i < 15; i++)
        if (!mc.count(keys[i])) throw KirError(std::string("machine: missing key ") + keys[i]);
    Machine m;
    m.issue_width = mc["issue_width"]; m.mem_ports = mc["mem_ports"]; m.registers = mc["registers"];
    m.reg_spill_penalty = mc["reg_spill_penalty"]; m.taken_branch_penalty = mc["taken_branch_penalty"];
    m.icache_size = mc["icache_size"]; m.icache_penalty = mc["icache_penalty"];
    m.lat_alu = mc["lat_alu"]; m.lat_mul = mc["lat_mul"]; m.lat_div = mc["lat_div"];
    m.lat_load = mc["lat_load"]; m.lat_mac = mc["lat_mac"]; m.lat_addsh = mc["lat_addsh"];
    m.fused_mac = mc["fused_mac"]; m.fused_addsh = mc["fused_addsh"];
    return m;
}

static inline void check_machine_features(const Program& p, const Machine& mc) {
    for (auto& b : p.blocks)
        for (auto& pi : b.ins) {
            if (pi.op == O_MAC && !mc.fused_mac) throw KirError("'mac' is not available on this machine");
            if (pi.op == O_ADDSH && !mc.fused_addsh) throw KirError("'addsh' is not available on this machine");
        }
}

// --------------------------------------------------------------------------
// Compiled form, execution, timing
// --------------------------------------------------------------------------
struct CInstr {
    Opc op;
    int d, x, y, z, t1, t2;
};
struct Compiled {
    std::vector<CInstr> code;
    std::vector<std::vector<int>> srcs, touch;
    std::vector<int> dst;
    int nregs = 0, nparams = 0;
    std::vector<u32> R0;
};

static inline Compiled compile_program(const Program& prog) {
    Compiled cp;
    std::unordered_map<ll, int> regmap;
    auto rid = [&](ll idx) {
        auto it = regmap.find(idx);
        if (it == regmap.end()) {
            int n = (int)regmap.size();
            regmap[idx] = n;
        }
    };
    for (ll p : prog.params) rid(p);
    std::unordered_map<std::string, int> labels;
    int idx = 0;
    for (auto& b : prog.blocks) {
        labels[b.label] = idx;
        idx += (int)b.ins.size();
        for (auto& pi : b.ins) {
            if (pi.d >= 0) rid(pi.d);
            for (ll x : pi.a)
                if (x >= 0) rid(x);
        }
    }
    int nregs = (int)regmap.size();
    std::unordered_map<ll, int> consts;
    std::vector<u32> cvals;
    auto slot = [&](ll x) -> int {
        if (x >= 0) return regmap[x];
        ll v = ~x;
        auto it = consts.find(v);
        if (it != consts.end()) return it->second;
        int s = nregs + (int)cvals.size();
        consts[v] = s;
        cvals.push_back((u32)v);
        return s;
    };
    for (auto& b : prog.blocks)
        for (auto& pi : b.ins) {
            CInstr c;
            c.op = pi.op;
            c.d = c.x = c.y = c.z = c.t1 = c.t2 = 0;
            std::vector<int> ss;
            int dest = -1;
            if (pi.d >= 0) { c.d = regmap[pi.d]; dest = c.d; }
            size_t na = pi.a.size();
            if (pi.op == O_ADDSH) {
                c.x = slot(pi.a[0]); c.y = slot(pi.a[1]); c.z = (int)(~pi.a[2]);
                ss.push_back(c.x); ss.push_back(c.y);
            } else {
                if (na > 0) { c.x = slot(pi.a[0]); ss.push_back(c.x); }
                if (na > 1) { c.y = slot(pi.a[1]); ss.push_back(c.y); }
                if (na > 2) { c.z = slot(pi.a[2]); ss.push_back(c.z); }
            }
            if (pi.nt > 0) c.t1 = labels[pi.t[0]];
            if (pi.nt > 1) c.t2 = labels[pi.t[1]];
            std::vector<int> sr, tc;
            for (int s : ss)
                if (s < nregs) sr.push_back(s);
            for (int s : sr)
                if (std::find(tc.begin(), tc.end(), s) == tc.end()) tc.push_back(s);
            if (dest >= 0 && std::find(tc.begin(), tc.end(), dest) == tc.end()) tc.push_back(dest);
            cp.code.push_back(c);
            cp.srcs.push_back(sr);
            cp.touch.push_back(tc);
            cp.dst.push_back(dest);
        }
    cp.nregs = nregs;
    cp.nparams = (int)prog.params.size();
    cp.R0.assign(nregs, 0);
    cp.R0.insert(cp.R0.end(), cvals.begin(), cvals.end());
    return cp;
}

struct Inputs {
    std::vector<u32> args;
    std::vector<u32> words;              // memory from address 0
    std::vector<u32> vin[4];             // queues of ports 0..3
};

enum Status { ST_RET, ST_TRAP_DIV, ST_TRAP_ADDR, ST_TRAP_EXPL, ST_TIMEOUT };
struct VEntry {
    char kind;
    u32 port, value;
    bool operator==(const VEntry& o) const { return kind == o.kind && port == o.port && value == o.value; }
};
struct Result {
    Status status = ST_TIMEOUT;
    u32 value = 0;
    std::vector<VEntry> vlog;
    std::vector<u32> mem;
    ll steps = 0;
    std::vector<int> trace;
};

static inline void execute(const Compiled& cp, const Inputs& inp, ll max_steps, bool want_trace, Result& res) {
    std::vector<u32> R(cp.R0);
    for (size_t i = 0; i < inp.args.size() && i < R.size(); i++) R[i] = inp.args[i];
    res.mem.assign(MEM_WORDS, 0);
    std::vector<u32>& mem = res.mem;
    for (size_t i = 0; i < inp.words.size() && i < (size_t)MEM_WORDS; i++) mem[i] = inp.words[i];
    size_t qpos[4] = {0, 0, 0, 0};
    res.vlog.clear();
    res.trace.clear();
    res.value = 0;
    const CInstr* code = cp.code.data();
    int pc = 0;
    ll steps = 0;
    while (true) {
        if (steps >= max_steps) { res.status = ST_TIMEOUT; res.steps = steps; return; }
        const CInstr& c = code[pc];
        steps++;
        if (want_trace) res.trace.push_back(pc);
        switch (c.op) {
            case O_ADD: R[c.d] = R[c.x] + R[c.y]; pc++; break;
            case O_SUB: R[c.d] = R[c.x] - R[c.y]; pc++; break;
            case O_MUL: R[c.d] = R[c.x] * R[c.y]; pc++; break;
            case O_AND: R[c.d] = R[c.x] & R[c.y]; pc++; break;
            case O_OR: R[c.d] = R[c.x] | R[c.y]; pc++; break;
            case O_XOR: R[c.d] = R[c.x] ^ R[c.y]; pc++; break;
            case O_SHL: R[c.d] = R[c.x] << (R[c.y] & 31); pc++; break;
            case O_SHR: R[c.d] = R[c.x] >> (R[c.y] & 31); pc++; break;
            case O_LT: R[c.d] = ((int32_t)R[c.x] < (int32_t)R[c.y]) ? 1 : 0; pc++; break;
            case O_EQ: R[c.d] = (R[c.x] == R[c.y]) ? 1 : 0; pc++; break;
            case O_CONST: case O_MOV: R[c.d] = R[c.x]; pc++; break;
            case O_SELECT: R[c.d] = R[c.x] != 0 ? R[c.y] : R[c.z]; pc++; break;
            case O_MAC: R[c.d] = R[c.x] * R[c.y] + R[c.z]; pc++; break;
            case O_ADDSH: R[c.d] = R[c.x] + (R[c.y] << c.z); pc++; break;
            case O_DIV: case O_MOD: {
                u32 y = R[c.y];
                if (y == 0) { res.status = ST_TRAP_DIV; res.steps = steps; return; }
                ll sx = (int32_t)R[c.x], sy = (int32_t)y;
                ll ax = sx < 0 ? -sx : sx, ay = sy < 0 ? -sy : sy;
                if (c.op == O_DIV) {
                    ll q = ax / ay;
                    if ((sx < 0) != (sy < 0)) q = -q;
                    R[c.d] = (u32)(q & MASK);
                } else {
                    ll r = ax % ay;
                    if (sx < 0) r = -r;
                    R[c.d] = (u32)(r & MASK);
                }
                pc++;
                break;
            }
            case O_LOAD: {
                u32 addr = R[c.x];
                if (addr >= (u32)MEM_WORDS) { res.status = ST_TRAP_ADDR; res.steps = steps; return; }
                R[c.d] = mem[addr];
                pc++;
                break;
            }
            case O_STORE: {
                u32 addr = R[c.x];
                if (addr >= (u32)MEM_WORDS) { res.status = ST_TRAP_ADDR; res.steps = steps; return; }
                mem[addr] = R[c.y];
                pc++;
                break;
            }
            case O_VLOAD: {
                u32 port = R[c.x];
                u32 val = 0;
                if (port < 4 && qpos[port] < inp.vin[port].size()) val = inp.vin[port][qpos[port]++];
                res.vlog.push_back(VEntry{'L', port, val});
                R[c.d] = val;
                pc++;
                break;
            }
            case O_VSTORE: res.vlog.push_back(VEntry{'S', R[c.x], R[c.y]}); pc++; break;
            case O_TRAP: res.status = ST_TRAP_EXPL; res.value = R[c.x]; res.steps = steps; return;
            case O_JMP: pc = c.t1; break;
            case O_BR: pc = R[c.x] != 0 ? c.t1 : c.t2; break;
            case O_RET: res.status = ST_RET; res.value = R[c.x]; res.steps = steps; return;
            default: throw KirError("bad opcode in execute");
        }
    }
}

// Exact cycle count for a finished run (KIR_SPEC section 10).
static inline ll time_trace(const Compiled& cp, const std::vector<int>& trace, const Machine& mc) {
    ll W = mc.issue_width, MP = mc.mem_ports, K = mc.registers;
    ll SP = mc.reg_spill_penalty, TB = mc.taken_branch_penalty;
    ll IC = mc.icache_size, ICP = mc.icache_penalty;
    size_t ni = cp.code.size();
    std::vector<ll> lat(ni);
    std::vector<char> isterm(ni), ismem(ni);
    for (size_t i = 0; i < ni; i++) {
        Opc o = cp.code[i].op;
        lat[i] = cp.dst[i] >= 0 ? mc.lat(o) : 1;
        isterm[i] = is_term(o);
        ismem[i] = is_memop(o);
    }
    std::vector<ll> ready(cp.nregs, 0);
    bool track = cp.nregs > K;
    std::list<int> order;                       // front = least recently used
    std::vector<std::list<int>::iterator> pos(cp.nregs);
    std::vector<char> state(cp.nregs, 0);       // 0 never seen, 1 resident, 2 spilled
    size_t nres = 0;
    if (track) {
        for (int r = 0; r < cp.nparams; r++) {
            if ((ll)nres >= K) {
                int v = order.front();
                order.pop_front();
                state[v] = 2;
                nres--;
            }
            order.push_back(r);
            pos[r] = std::prev(order.end());
            state[r] = 1;
            nres++;
        }
    }
    ll last = 0, used = 0, memused = 0, fence = 0;
    size_t n = trace.size();
    for (size_t idx = 0; idx < n; idx++) {
        int pc = trace[idx];
        ll F = pc >= IC ? ICP : 0;
        if (track) {
            for (int r : cp.touch[pc]) {
                if (state[r] == 1) {
                    order.splice(order.end(), order, pos[r]);
                } else {
                    if (state[r] == 2) F += SP;
                    if ((ll)nres >= K) {
                        int v = order.front();
                        order.pop_front();
                        state[v] = 2;
                        nres--;
                    }
                    order.push_back(r);
                    pos[r] = std::prev(order.end());
                    state[r] = 1;
                    nres++;
                }
            }
        }
        ll t = last + F;
        if (fence > t) t = fence;
        for (int r : cp.srcs[pc])
            if (ready[r] > t) t = ready[r];
        int d = cp.dst[pc];
        if (d >= 0 && ready[d] > t) t = ready[d];
        bool m = ismem[pc];
        if (t == last && (used >= W || (m && memused >= MP))) t += 1;
        if (t > last) { used = 0; memused = 0; }
        used++;
        if (m) memused++;
        last = t;
        if (d >= 0) ready[d] = t + lat[pc];
        if (isterm[pc] && idx + 1 < n && trace[idx + 1] != pc + 1) fence = t + 1 + TB;
    }
    return last + 1;
}

static inline bool same_behaviour(const Result& a, const Result& b) {
    if (a.status != b.status) return false;
    if ((a.status == ST_RET || a.status == ST_TRAP_EXPL) && a.value != b.value) return false;
    if (a.vlog.size() != b.vlog.size()) return false;
    for (size_t i = 0; i < a.vlog.size(); i++)
        if (!(a.vlog[i] == b.vlog[i])) return false;
    for (int i = 0; i < SCRATCH_BASE; i++)
        if (a.mem[i] != b.mem[i]) return false;
    return true;
}

// splitmix64, identical to the kit's generator
struct Rng {
    ull s;
    explicit Rng(ull seed) : s(seed) {}
    ull next() {
        s += 0x9E3779B97F4A7C15ULL;
        ull z = s;
        z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ULL;
        z = (z ^ (z >> 27)) * 0x94D049BB133111EBULL;
        return z ^ (z >> 31);
    }
    ull below(ull n) { return next() % n; }
};

static const u32 EDGE_VALUES[20] = {0, 1, 2, 3, 7, 8, 15, 16, 31, 32, 63, 64, 255, 256, 1000, 65535,
                                    0x7FFFFFFFu, 0x80000000u, 0xFFFFFFFEu, 0xFFFFFFFFu};
static const u32 SPECIAL_PAIRS[16][2] = {
    {0x80000000u, 0xFFFFFFFFu}, {0xFFFFFFFFu, 0x80000000u}, {0x7FFFFFFFu, 0xFFFFFFFFu}, {1, 0},
    {0, 1}, {0xFFFFFFFFu, 1}, {7, 0xFFFFFFFEu}, {0xFFFFFFF9u, 2}, {7, 2}, {0x80000000u, 1},
    {0x80000000u, 2}, {0x7FFFFFFFu, 1}, {0x80000000u, 31}, {1, 32}, {1, 33}, {0xFFFFFFFFu, 31}};

// The kit's generate_inputs (same values for the same seed).
static inline std::vector<Inputs> generate_inputs(int nparams, int count, ull seed) {
    Rng rng(seed);
    std::vector<Inputs> out;
    for (int i = 0; i < count; i++) {
        Inputs in;
        if (i < 20) {
            in.args.assign(nparams, EDGE_VALUES[i]);
        } else if (i < 36) {
            for (int j = 0; j < nparams; j++)
                in.args.push_back(j < 2 ? SPECIAL_PAIRS[i - 20][j] : EDGE_VALUES[(i + j) % 20]);
        } else {
            for (int j = 0; j < nparams; j++) {
                ull r = rng.below(10);
                if (r < 6) in.args.push_back((u32)rng.below(16));
                else if (r < 8) in.args.push_back(EDGE_VALUES[rng.below(20)]);
                else in.args.push_back((u32)(rng.next() & 0xFFFFFFFFULL));
            }
        }
        ull mode = rng.below(4);
        static const ull bounds[4] = {16, 1000, 0, 256};
        ull bound = bounds[mode];
        for (int w = 0; w < 256; w++)
            in.words.push_back(bound == 0 ? (u32)(rng.next() & 0xFFFFFFFFULL) : (u32)rng.below(bound));
        for (int port = 0; port < 4; port++)
            for (int k = 0; k < 16; k++) in.vin[port].push_back((u32)rng.below(1000));
        out.push_back(in);
    }
    return out;
}
