a = echo("lib a") 1;
$fn = 50;
r = rands(0, 1, 1, 7)[0];
module lm() { echo("lm", a, $fn); sphere(); }
module lc() { echo("lc", $fn); children(); }
function lf() = [a, $fn];
function lclosure() = function() a * 10;
w = $fa;
function lw() = w;
cube(100);
echo("never printed");
