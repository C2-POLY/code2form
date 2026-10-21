function segs(r, fn = $fn) = fn > 0 ? fn : 5;
module c() echo(segs(1));
c($fn = 20);
module ring(d = $size) echo("ring", d);
module holder() { $size = 7; ring(); }
holder();
g = function(a = $d) a;
echo(let($d = 1) g());
module mx($x = 5, y = $x) echo("mx", $x, y);
mx($x = 1);
function f(n, acc = []) = n == 0 ? acc : f(n - 1, concat(acc, [function() n]));
echo([for (g = f(3)) g()]);
function f4(n, acc = []) = n == 0 ? acc : f4(n - 1, concat(acc, [function() extra]), extra = n);
echo([for (g = f4(3)) g()]);
y = 1;
function h(n) = n == 0 ? y : h(n - 1);
echo(h(3, y = 5));
z = 42;
function h2(n) = n == 0 ? z : n == 2 ? h2(n - 1, z = 7) : h2(n - 1);
echo(h2(3));
fs = [for (i = [0:2]) function() i];
echo([for (f = fs) f()]);
echo([for (f = [for (i = 0; i < 3; i = i + 1) function() i]) f()]);
echo([for (f = [for (i = [0:1], j = [0:1]) function() [i, j]]) f()]);
echo([for (f = [for (c = "abc") function() c]) f()]);
echo([for (f = [for (i = [0:2]) function(x = i) x]) f()]);
echo([for (f = [for (i = [0:2]) let(j = i) function() j]) f()]);
function fa(a = echo("default a") 1) = a;
echo(fa(5));
module m2(p = echo("pdef") 0) echo("m2", p);
m2(7);
for ([0:2]) cube();
echo([for ([0:2]) 1]);
intersection_for ([0:1]) sphere();
echo([for (i = [0:1], [5:6]) i]);
echo([for (a = 1, b = 1; a < 50; a = b, b = a + b) a]);
echo([for (i = 0, s = 0; i < 5; i = i + 1, s = s + i) [i, s]]);
echo([for (i = 0; i < 3; i = i + 1, j = i * 10) [i, j]]);
echo([for (i = 0, j = 0; i < 3; j = i, i = i + 1, j = j + 100) [i, j]]);
$v = 1;
module m(p) { $v = 5; echo("body"); children(); children(0); }
m(echo("arg") 1) { x = echo("assign") $v; echo("child", x); }
module k() echo("k");
k() { zz = echo("assign k") 1; cube(); }
echo("x") { yy = echo("assign echo") 1; }
assert(true) { w = echo("assign assert") 1; }
