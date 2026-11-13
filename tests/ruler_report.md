# Ruler report (tests/ruler.py)

IPC = dynamic instructions / cycles of the kopt output on the program's .in. LB = max(dataflow critical path with infinite width and no penalties, ceil(n/W), ceil(mem/ports)). gap = cycles/LB. Full run: 100 bench programs x 5 machines (500 rows): 24 at issue bound, 169 at latency bound (within 10% of LB), 36 branch-penalty dominated, 270 other (LB is a max, not a sum, so mixed issue+latency cost shows as a gap of 1.2-2x). No timeouts or errors.

Mean IPC: default machine (W=2) 1.12; wide_fast (W=4) 1.56. Worst gaps on default: bubble_sort 2.29, insertion_sort 1.93, sensor_threshold 1.90, dfa_dispatch 1.81, rle_encode 1.76 (loop-carried memory/branch chains, taken-branch penalty).

## large_chain (100k instrs, tests/gen_large.py --only chain)

Verdict: Q-bound. On the default machine (W=2, C=64, Q=2) it takes 206,527 cycles for 100,001 instructions (IPC 0.48 vs W=2). Re-timing the same trace with icache_penalty=0 gives 113,173 cycles, exactly the dataflow critical path, so without Q it would be purely latency-bound (the chain is serial; the issue bound is only 50,001). Q adds about 93,000 cycles (45%): nearly every instruction lies beyond C=64 and pays Q fetch cycles. The chain runs straight-line, each instruction once, so removing instructions is the only lever. Other machines: slow_memory 219,699, wide_fast 199,941 (width does not help; Q does).

## Table, default machine (W=2)

```
prog               machine        W        n   cycles   IPC critpath  issueLB      LB   gap  limit
addsh_index        default        2      135       84  1.61       78       68      78  1.08  at-bound(latency)
adler32            default        2      171      106  1.61      100       86     100  1.06  at-bound(latency)
alias_hazard       default        2        9       13  0.69        8        5       8  1.62  stall/spill
alias_stores       default        2      188      147  1.28       78       94      94  1.56  stall/spill
array_max          default        2       72       49  1.47       28       36      36  1.36  stall/spill
array_min          default        2       66       44  1.50       26       33      33  1.33  stall/spill
array_sum          default        2       57       45  1.27       23       29      29  1.55  stall/spill
binary_search      default        2       52       44  1.18       43       26      43  1.02  at-bound(latency)
bit_reverse        default        2      261      146  1.79       92      131     131  1.11  stall/spill
bounds_trap        default        2      399      256  1.56      166      200     200  1.28  stall/spill
branchy_select     default        2      295      160  1.84      108      148     148  1.08  at-bound(issue)
bubble_sort        default        2      468      536  0.87      219      234     234  2.29  stall/spill
checked_add_trap   default        2        4        3  1.33        3        2       3  1.00  at-bound(latency)
collatz            default        2     1061      616  1.72      448      531     531  1.16  stall/spill
common_subexpr     default        2        8       10  0.80       10        4      10  1.00  at-bound(latency)
const_chain        default        2        1        1  1.00        1        1       1  1.00  at-bound(issue)
constant_branch    default        2        8        9  0.89        9        4       9  1.00  at-bound(latency)
conv_mac           default        2      233      274  0.85      183      117     183  1.50  stall/spill
count_above        default        2      119       71  1.68       69       60      69  1.03  at-bound(latency)
count_divisors     default        2      375      444  0.84      376      188     376  1.18  stall/spill
crc_bitwise        default        2      174      191  0.91      160       87     160  1.19  stall/spill
dead_div_trap      default        2        3       14  0.21       14        2      14  1.00  at-bound(latency)
dead_load_trap     default        2        4        2  2.00        2        2       2  1.00  at-bound(issue)
dead_mod_in_loop   default        2       36       51  0.71       44       18      44  1.16  stall/spill
dead_overwritten_stores default        2      134       78  1.72       23       67      67  1.16  stall/spill
dfa_dispatch       default        2      284      257  1.11      134      142     142  1.81  stall/spill
digit_sum          default        2       37       60  0.62       55       19      55  1.09  at-bound(latency)
div_trap           default        2        4       14  0.29       14        2      14  1.00  at-bound(latency)
dot_pairs          default        2       73       50  1.46       33       37      37  1.35  stall/spill
dot_product        default        2       67       44  1.52       32       34      34  1.29  stall/spill
factorial_mod      default        2      108      303  0.36      302       54     302  1.00  at-bound(latency)
fib                default        2       71       45  1.58       35       36      36  1.25  stall/spill
fletcher16         default        2      111       76  1.46       70       56      70  1.09  at-bound(latency)
fnv_hash           default        2       98       91  1.08       74       49      74  1.23  stall/spill
gcd                default        2       13       41  0.32       38        7      38  1.08  at-bound(latency)
gray_code_sum      default        2      189      107  1.77       55       95      95  1.13  stall/spill
guarded_div        default        2       40      101  0.40       88       20      88  1.15  stall/spill
hamming_dist       default        2      176      131  1.34       88       88      88  1.49  stall/spill
histogram          default        2      278      251  1.11      192      139     192  1.31  stall/spill
horner             default        2       17       27  0.63       26        9      26  1.04  at-bound(latency)
index_trap         default        2        4        2  2.00        2        2       2  1.00  at-bound(issue)
insertion_sort     default        2      388      375  1.03       95      194     194  1.93  stall/spill
irreducible_loop   default        2       88       48  1.83       42       44      44  1.09  at-bound(issue)
is_prime           default        2       18       30  0.60       27        9      27  1.11  stall/spill
isqrt              default        2      130      102  1.27      101       65     101  1.01  at-bound(latency)
jump_chain         default        2        8        9  0.89        9        4       9  1.00  at-bound(latency)
junk_dead          default        2       64       54  1.19       44       32      44  1.23  stall/spill
lcm_pair           default        2       22       70  0.31       56       11      56  1.25  stall/spill
log_then_divide    default        2        6       15  0.40       13        3      13  1.15  stall/spill
loop_invariant     default        2       11       12  0.92       12        6      12  1.00  at-bound(latency)
many_live          default        2       26       16  1.62       14       13      14  1.14  stall/spill
matmul4            default        2      182      384  0.47      246       91     246  1.56  stall/spill
matrix_add4        default        2       66       55  1.20       23       33      48  1.15  stall/spill
matvec8            default        2       55       61  0.90       44       28      44  1.39  stall/spill
merge_sorted       default        2      229      217  1.06      136      115     136  1.60  stall/spill
mini_interp        default        2      134      112  1.20       63       67      67  1.67  stall/spill
multi_exit         default        2      310      232  1.34      104      155     155  1.50  stall/spill
nested_invariant   default        2       13       13  1.00       13        7      13  1.00  at-bound(latency)
paren_depth        default        2      169      166  1.02      103       85     103  1.61  stall/spill
parity_fold        default        2       12       12  1.00       12        6      12  1.00  at-bound(latency)
partial_redundancy default        2        6        6  1.00        6        3       6  1.00  at-bound(latency)
poly_mac           default        2        4       10  0.40       10        2      10  1.00  at-bound(latency)
popcount           default        2      143       79  1.81       40       72      72  1.10  at-bound(issue)
pow_fast           default        2       68      125  0.54      124       34     124  1.01  at-bound(latency)
powmod             default        2       46       99  0.46       94       23      94  1.05  at-bound(latency)
prefix_sum         default        2       51       32  1.59       19       26      26  1.23  stall/spill
redundant_loads    default        2      104       83  1.25       51       52      52  1.60  stall/spill
reverse_array      default        2       38       31  1.23       19       19      19  1.63  stall/spill
rle_encode         default        2      130      127  1.02       72       65      72  1.76  stall/spill
rotate_array       default        2       55       47  1.17       25       28      28  1.68  stall/spill
rotate_mix         default        2       66       37  1.78       27       33      33  1.12  stall/spill
row_sums           default        2       58       44  1.32       29       29      29  1.52  stall/spill
run_count          default        2      108       76  1.42       41       54      54  1.41  stall/spill
sem_bit_mix        default        2       10        6  1.67        5        5       5  1.20  stall/spill
sem_div_mod        default        2       10       17  0.59       17        5      17  1.00  at-bound(latency)
sem_mul_wrap       default        2        7       20  0.35       20        4      20  1.00  at-bound(latency)
sem_select_abs     default        2       11       15  0.73       15        6      15  1.00  at-bound(latency)
sem_shifts         default        2       10        6  1.67        5        5       5  1.20  stall/spill
sem_signed_cmp     default        2        7        6  1.17        6        4       6  1.00  at-bound(latency)
sensor_drain       default        2       36       31  1.16       25       18      25  1.24  stall/spill
sensor_sum         default        2       48       41  1.17       22       24      24  1.71  stall/spill
sensor_threshold   default        2       68       97  0.70       51       34      51  1.90  branch
sieve              default        2     1297     1187  1.09      730      649     730  1.63  stall/spill
spill_storm        default        2      696      946  0.74      770      348     770  1.23  stall/spill
stencil3           default        2      204      131  1.56       89      102     102  1.28  stall/spill
store_then_div_trap default        2        5       14  0.36       13        3      13  1.08  at-bound(latency)
strength_ladder    default        2      527      282  1.87      253      264     264  1.07  at-bound(issue)
sum_cubes_mod      default        2       18       27  0.67       26        9      26  1.04  at-bound(latency)
sum_sq             default        2       11       15  0.73       15        6      15  1.00  at-bound(latency)
swap_via_memory    default        2        9       15  0.60        9        5       9  1.67  stall/spill
transpose8         default        2      284      345  0.82      237      142     237  1.46  stall/spill
unreachable_blocks default        2        3        3  1.00        3        2       3  1.00  at-bound(latency)
unrolled_crc12     default        2       37       38  0.97       26       19      26  1.46  stall/spill
unrolled_poly      default        2       48       75  0.64       74       24      74  1.01  at-bound(latency)
vec_add            default        2       80       46  1.74       28       40      40  1.15  stall/spill
vending_fsm        default        2      214      261  0.82      176      107     176  1.48  stall/spill
vlog_every_other   default        2       46       29  1.59       11       23      23  1.26  stall/spill
wide_expression    default        2       18       13  1.38       12        9      12  1.08  at-bound(latency)
word_count         default        2      188      101  1.86       64       94      94  1.07  at-bound(issue)
xorshift           default        2      185      131  1.41      121       93     121  1.08  at-bound(latency)
```
