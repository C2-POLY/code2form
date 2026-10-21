assign(a = 2) cube(a);
module m() child(0);
m() sphere(1);
import_stl("nofile.stl");
import_dxf("nofile.dxf");
dxf_linear_extrude(file = "nofile.dxf", height = 2);
x = 10; assign(x = 1, y = x) echo(x, y); module m2() { child(); child(5); } m2() cube(3); import_off("nofile.off"); dxf_rotate_extrude(file = "nofile.dxf");
difference($fn = 50) { cylinder(r = 10, h = 5); cylinder(r = 8, h = 6); }
union($fa=1, $fs=0.5) sphere(5);
group($fn=7) circle(2);
intersection($fn=9) circle(2);
hull($fn=11) circle(2);
union($x=5) echo($x); difference($x=7) { echo($x); } render($x=6) echo($x);
