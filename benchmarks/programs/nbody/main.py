import sys, math

PI = 3.141592653589793
SOLAR_MASS = 4.0 * PI * PI
DAYS_PER_YEAR = 365.24

bodies = [
    [0,0,0,0,0,0,SOLAR_MASS],
    [4.84143144246472090e+00,-1.16032004402742839e+00,-1.03622044471123109e-01,
     1.66007664274403694e-03*DAYS_PER_YEAR,7.69901118419740425e-03*DAYS_PER_YEAR,
     -6.90460016972063023e-05*DAYS_PER_YEAR,9.54791938424326609e-04*SOLAR_MASS],
    [8.34336671824457987e+00,4.12479856412430479e+00,-4.03523417114321381e-01,
     -2.76742510726862411e-03*DAYS_PER_YEAR,4.99852801234917238e-03*DAYS_PER_YEAR,
     2.30417297573763929e-05*DAYS_PER_YEAR,2.85885980666130812e-04*SOLAR_MASS],
    [1.28943695621391310e+01,-1.51111514016986312e+01,-2.23307578892655734e-01,
     2.96460137564761618e-03*DAYS_PER_YEAR,2.37847173959480950e-03*DAYS_PER_YEAR,
     -2.96589568540237556e-05*DAYS_PER_YEAR,4.36624404335156298e-05*SOLAR_MASS],
    [1.53796971148509165e+01,-2.59193146099879641e+01,1.79258772950371181e-01,
     2.68067772490389322e-03*DAYS_PER_YEAR,1.62824170038242295e-03*DAYS_PER_YEAR,
     -9.51592254519715870e-05*DAYS_PER_YEAR,5.15138902046611451e-05*SOLAR_MASS]
]

def offset_momentum():
    px=py=pz=0.0
    for b in bodies: px+=b[3]*b[6]; py+=b[4]*b[6]; pz+=b[5]*b[6]
    bodies[0][3]=-px/SOLAR_MASS; bodies[0][4]=-py/SOLAR_MASS; bodies[0][5]=-pz/SOLAR_MASS

def energy():
    e=0.0
    for i in range(5):
        b=bodies[i]
        e+=0.5*b[6]*(b[3]*b[3]+b[4]*b[4]+b[5]*b[5])
        for j in range(i+1,5):
            c=bodies[j]
            dx,dy,dz=b[0]-c[0],b[1]-c[1],b[2]-c[2]
            e-=b[6]*c[6]/math.sqrt(dx*dx+dy*dy+dz*dz)
    return e

def advance(dt):
    for i in range(5):
        bi=bodies[i]
        for j in range(i+1,5):
            bj=bodies[j]
            dx,dy,dz=bi[0]-bj[0],bi[1]-bj[1],bi[2]-bj[2]
            dist=math.sqrt(dx*dx+dy*dy+dz*dz)
            mag=dt/(dist*dist*dist)
            bi[3]-=dx*bj[6]*mag; bi[4]-=dy*bj[6]*mag; bi[5]-=dz*bj[6]*mag
            bj[3]+=dx*bi[6]*mag; bj[4]+=dy*bi[6]*mag; bj[5]+=dz*bi[6]*mag
    for b in bodies: b[0]+=dt*b[3]; b[1]+=dt*b[4]; b[2]+=dt*b[5]

n=int(sys.argv[1])
offset_momentum()
print(f"{energy():.9f}")
for _ in range(n): advance(0.01)
print(f"{energy():.9f}")
