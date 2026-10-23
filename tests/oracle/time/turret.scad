// $t, the animation's time: each golden is this file at one time.
echo(t = $t);
module blade(angle = 360 * $t) rotate([0, 0, angle]) cube([20, 2, 1]);
cube([40, 40, 5]);
translate([20, 20, 5]) rotate([0, 0, 360 * $t]) {
    cylinder(h = 10, r = 12);
    translate([0, 8, 5 + 3 * sin(360 * $t)]) rotate([-90, 0, 0]) cylinder(h = 25, r = 2);
}
translate([0, 0, 20]) blade();
if ($t >= 0.5) translate([60, 0, 0]) sphere(5);
