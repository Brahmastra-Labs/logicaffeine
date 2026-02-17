import os, strutils, math

const
  PI = 3.141592653589793
  SOLAR_MASS = 4.0 * PI * PI
  DAYS_PER_YEAR = 365.24

type Body = object
  x, y, z, vx, vy, vz, mass: float64

var bodies: array[5, Body] = [
  Body(x:0, y:0, z:0, vx:0, vy:0, vz:0, mass:SOLAR_MASS),
  Body(x:4.84143144246472090e+00, y: -1.16032004402742839e+00, z: -1.03622044471123109e-01,
    vx:1.66007664274403694e-03*DAYS_PER_YEAR, vy:7.69901118419740425e-03*DAYS_PER_YEAR,
    vz: -6.90460016972063023e-05*DAYS_PER_YEAR, mass:9.54791938424326609e-04*SOLAR_MASS),
  Body(x:8.34336671824457987e+00, y:4.12479856412430479e+00, z: -4.03523417114321381e-01,
    vx: -2.76742510726862411e-03*DAYS_PER_YEAR, vy:4.99852801234917238e-03*DAYS_PER_YEAR,
    vz:2.30417297573763929e-05*DAYS_PER_YEAR, mass:2.85885980666130812e-04*SOLAR_MASS),
  Body(x:1.28943695621391310e+01, y: -1.51111514016986312e+01, z: -2.23307578892655734e-01,
    vx:2.96460137564761618e-03*DAYS_PER_YEAR, vy:2.37847173959480950e-03*DAYS_PER_YEAR,
    vz: -2.96589568540237556e-05*DAYS_PER_YEAR, mass:4.36624404335156298e-05*SOLAR_MASS),
  Body(x:1.53796971148509165e+01, y: -2.59193146099879641e+01, z:1.79258772950371181e-01,
    vx:2.68067772490389322e-03*DAYS_PER_YEAR, vy:1.62824170038242295e-03*DAYS_PER_YEAR,
    vz: -9.51592254519715870e-05*DAYS_PER_YEAR, mass:5.15138902046611451e-05*SOLAR_MASS)
]

proc offsetMomentum() =
  var px, py, pz: float64
  for i in 0..4: px+=bodies[i].vx*bodies[i].mass; py+=bodies[i].vy*bodies[i].mass; pz+=bodies[i].vz*bodies[i].mass
  bodies[0].vx = -px/SOLAR_MASS; bodies[0].vy = -py/SOLAR_MASS; bodies[0].vz = -pz/SOLAR_MASS

proc energy(): float64 =
  var e = 0.0
  for i in 0..4:
    e += 0.5*bodies[i].mass*(bodies[i].vx*bodies[i].vx+bodies[i].vy*bodies[i].vy+bodies[i].vz*bodies[i].vz)
    for j in (i+1)..4:
      let dx=bodies[i].x-bodies[j].x; let dy=bodies[i].y-bodies[j].y; let dz=bodies[i].z-bodies[j].z
      e -= bodies[i].mass*bodies[j].mass/sqrt(dx*dx+dy*dy+dz*dz)
  e

proc advance(dt: float64) =
  for i in 0..4:
    for j in (i+1)..4:
      let dx=bodies[i].x-bodies[j].x; let dy=bodies[i].y-bodies[j].y; let dz=bodies[i].z-bodies[j].z
      let dist=sqrt(dx*dx+dy*dy+dz*dz)
      let mag=dt/(dist*dist*dist)
      bodies[i].vx-=dx*bodies[j].mass*mag; bodies[i].vy-=dy*bodies[j].mass*mag; bodies[i].vz-=dz*bodies[j].mass*mag
      bodies[j].vx+=dx*bodies[i].mass*mag; bodies[j].vy+=dy*bodies[i].mass*mag; bodies[j].vz+=dz*bodies[i].mass*mag
  for i in 0..4:
    bodies[i].x+=dt*bodies[i].vx; bodies[i].y+=dt*bodies[i].vy; bodies[i].z+=dt*bodies[i].vz

let n = parseInt(paramStr(1))
offsetMomentum()
echo formatFloat(energy(), ffDecimal, 9)
for i in 0..<n: advance(0.01)
echo formatFloat(energy(), ffDecimal, 9)
