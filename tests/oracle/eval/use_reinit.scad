use <libs/counter.scad>
echo("main");
lm(); lm();
echo(lf(), lf());
lm($fn = 10);
translate([0, 0, 0], $fn = 7) lm();
lc() sphere();
c = lclosure(); echo(c(), c());
echo(let($fa = 5) lw());
mf = lf(); echo(mf);
