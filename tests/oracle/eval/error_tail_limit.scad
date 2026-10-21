echo("before");
function t(n, acc = 0) = n == 0 ? acc : t(n - 1, acc + 1);
echo(t(1000001));
echo("after");
cube(1);
