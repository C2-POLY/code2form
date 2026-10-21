include <libs/self_inc.scad>
x = 1 + include <libs/three.scad> 2;
echo(x);
include <libs/sub/a.scad>
in_a();
include <libs/sub/uses.scad>
from_sub_lib();
module scoped() { include <libs/sub/b.scad> echo("module sees", bval); }
scoped();
include <no_such_file_xyz.scad>
echo("after missing");
