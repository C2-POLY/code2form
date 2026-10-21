module e() {} e(); module f() { echo("in f"); } f(); echo("x") cube(); assert(true) cube(); translate([1,0]);
module m() { echo($children); children(0); children([0,2]); children([1:2]); children(5); } m() { cube(1); sphere(2); cylinder(); }
module p() { echo(parent_module(0), parent_module(1), $parent_modules); } module q() p(); q();
module m2() { $fn = 7; children(); } m2() sphere(); module n() sphere(); $fn=3; n(); module k($fn=9) sphere(); k();
echo("a"); for (i=[]) cube(); if (false) cube(); let(a=1); assert(true); children(); intersection_for(i=[]) cube(); for(i=[1]); if (true); module m3() children(); m3(); union(); group();
module wrap() { translate([1,0,0]) children(); } module twice() { children(); children(); } twice() wrap() cube();
module outer() inner() children(); module inner() { echo(inner_children = $children); children(); } outer() { sphere(); cube(); }
module withvars() { children(); } withvars() { v = 4; cube(v); }
#translate([1,0,0]) cube(); %group() cube(); module mm() cube(); #mm(); #%cube(); %#cube();
