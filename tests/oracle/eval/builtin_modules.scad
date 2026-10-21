cube(); cube(5); cube([1,2,3], center=true); cube(size=[1,2]);
sphere(); sphere(2); sphere(d=3, $fn=10);
cylinder(); cylinder(h=2, r=1); cylinder(h=3, r1=1, r2=2, center=true); cylinder(d=4, h=1); cylinder(d1=1,d2=2);
polyhedron(points=[[0,0,0],[1,0,0],[0,1,0],[0,0,1]], faces=[[0,1,2],[0,1,3],[0,2,3],[1,2,3]]);
square(); square(2, center=true); square([1,2]);
circle(); circle(2); circle(d=3);
polygon([[0,0],[1,0],[0,1]]); polygon(points=[[0,0],[1,0],[0,1],[0.2,0.2]], paths=[[0,1,2],[3]]);
text("hi");
translate([1,2,3]) cube(); rotate([10,20,30]) cube(); rotate(45) cube(); rotate(30, [1,1,0]) cube(); scale(2) cube(); scale([1,2,3]) cube();
mirror([1,0,0]) cube(); multmatrix([[1,0,0,1],[0,1,0,2],[0,0,1,3]]) cube(); resize([1,2,3]) cube(); color("red") cube(); color([1,0,0,0.5]) cube(); color("#ff0000") cube(); color("blue", 0.3) cube();
offset(r=1) square(); offset(delta=1, chamfer=true) square();
union() { cube(); sphere(); } difference() { cube(); sphere(); } intersection() { cube(); sphere(); }
hull() { cube(); sphere(); } minkowski() { cube(); sphere(); }
linear_extrude(height=10, twist=90, slices=5, scale=2, center=true) square(); rotate_extrude(angle=180) square(); rotate_extrude() square();
projection(cut=true) cube(); render(convexity=2) cube(); surface(file="x.dat");
import("x.stl");
for (i=[1:2]) cube(i); intersection_for(i=[1:2]) cube(i); if (true) cube(); else sphere(); let(a=1) cube(a);
echo("x"); group() cube();
module m() { children(); } m() { cube(); sphere(); }
#cube(); %cube(); *cube();
