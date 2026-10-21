// Parametric storage box
/* [Box] */
// Outer width in mm
width = 60; // [20:200]
// Outer depth in mm
depth = 40; // [20:200]
height = 25; // [10:100]
wall = 2; // [1:0.5:5]
/* [Lid] */
lid = true;
style = "rounded"; // [rounded, square:Square corners]
/* [Hidden] */
$fn = 48;
eps = 0.01;

function inner(v) = [for (x = v) x - 2 * wall];

module rounded_square(size, r) {
    offset(r = r) offset(delta = -r) square(size, center = true);
}

module shell(size) {
    difference() {
        linear_extrude(size.z) rounded_square([size.x, size.y], 3);
        translate([0, 0, wall]) linear_extrude(size.z) rounded_square(inner([size.x, size.y]), 1);
    }
}

shell([width, depth, height]);
if (lid)
    translate([width + 5, 0, 0])
        linear_extrude(wall) rounded_square([width, depth], 3);
