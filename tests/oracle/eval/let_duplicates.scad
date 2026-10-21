echo(let(a = 1, a = 2) a);
echo([for (i = [0:1]) let(i = i + 1, i = i * 10) i]);
let(a = 1, b = a, a = 3) echo(a, b);
function g(n) = n == 0 ? 0 : let(n = n - 1, n = 100) g(n);
echo(g(3));
