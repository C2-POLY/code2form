function c(n, acc) = n == 0 ? acc : c(n - 1, concat(acc, [n]));
echo(len(c(100000, [])));
function dbl(v, n) = n == 0 ? v : dbl(concat(v, v), n - 1);
echo(len(dbl([1], 24)));
x = dbl([1,2], 10); echo(x[1023], x[2047], len(x));
function c2(n, acc) = n == 0 ? acc : c2(n - 1, [each acc, n]);
echo(len(c2(50000, [])), c2(40, [])[39]);
v = concat([for (i = [0:99]) i], [for (i = [0:99]) -i]); echo(v[150], len(v), [each v][199]);
