function a(n) = n == 0 ? 0 : b(n - 1);
function b(n) = n == 0 ? 0 : a(n - 1);
echo(a(300000));
cube(1);
