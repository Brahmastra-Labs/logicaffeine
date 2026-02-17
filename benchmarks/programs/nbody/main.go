package main

import (
	"fmt"
	"math"
	"os"
	"strconv"
)

const PI = 3.141592653589793
const SOLAR_MASS = 4.0 * PI * PI
const DAYS_PER_YEAR = 365.24

type Body struct {
	x, y, z, vx, vy, vz, mass float64
}

var bodies = [5]Body{
	{0, 0, 0, 0, 0, 0, SOLAR_MASS},
	{4.84143144246472090e+00, -1.16032004402742839e+00, -1.03622044471123109e-01,
		1.66007664274403694e-03 * DAYS_PER_YEAR, 7.69901118419740425e-03 * DAYS_PER_YEAR,
		-6.90460016972063023e-05 * DAYS_PER_YEAR, 9.54791938424326609e-04 * SOLAR_MASS},
	{8.34336671824457987e+00, 4.12479856412430479e+00, -4.03523417114321381e-01,
		-2.76742510726862411e-03 * DAYS_PER_YEAR, 4.99852801234917238e-03 * DAYS_PER_YEAR,
		2.30417297573763929e-05 * DAYS_PER_YEAR, 2.85885980666130812e-04 * SOLAR_MASS},
	{1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01,
		2.96460137564761618e-03 * DAYS_PER_YEAR, 2.37847173959480950e-03 * DAYS_PER_YEAR,
		-2.96589568540237556e-05 * DAYS_PER_YEAR, 4.36624404335156298e-05 * SOLAR_MASS},
	{1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01,
		2.68067772490389322e-03 * DAYS_PER_YEAR, 1.62824170038242295e-03 * DAYS_PER_YEAR,
		-9.51592254519715870e-05 * DAYS_PER_YEAR, 5.15138902046611451e-05 * SOLAR_MASS},
}

func offsetMomentum() {
	var px, py, pz float64
	for i := 0; i < 5; i++ {
		px += bodies[i].vx * bodies[i].mass
		py += bodies[i].vy * bodies[i].mass
		pz += bodies[i].vz * bodies[i].mass
	}
	bodies[0].vx = -px / SOLAR_MASS
	bodies[0].vy = -py / SOLAR_MASS
	bodies[0].vz = -pz / SOLAR_MASS
}

func energy() float64 {
	e := 0.0
	for i := 0; i < 5; i++ {
		e += 0.5 * bodies[i].mass * (bodies[i].vx*bodies[i].vx + bodies[i].vy*bodies[i].vy + bodies[i].vz*bodies[i].vz)
		for j := i + 1; j < 5; j++ {
			dx := bodies[i].x - bodies[j].x
			dy := bodies[i].y - bodies[j].y
			dz := bodies[i].z - bodies[j].z
			e -= bodies[i].mass * bodies[j].mass / math.Sqrt(dx*dx+dy*dy+dz*dz)
		}
	}
	return e
}

func advance(dt float64) {
	for i := 0; i < 5; i++ {
		for j := i + 1; j < 5; j++ {
			dx := bodies[i].x - bodies[j].x
			dy := bodies[i].y - bodies[j].y
			dz := bodies[i].z - bodies[j].z
			dist := math.Sqrt(dx*dx + dy*dy + dz*dz)
			mag := dt / (dist * dist * dist)
			bodies[i].vx -= dx * bodies[j].mass * mag
			bodies[i].vy -= dy * bodies[j].mass * mag
			bodies[i].vz -= dz * bodies[j].mass * mag
			bodies[j].vx += dx * bodies[i].mass * mag
			bodies[j].vy += dy * bodies[i].mass * mag
			bodies[j].vz += dz * bodies[i].mass * mag
		}
	}
	for i := 0; i < 5; i++ {
		bodies[i].x += dt * bodies[i].vx
		bodies[i].y += dt * bodies[i].vy
		bodies[i].z += dt * bodies[i].vz
	}
}

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	offsetMomentum()
	fmt.Printf("%.9f\n", energy())
	for i := int64(0); i < n; i++ {
		advance(0.01)
	}
	fmt.Printf("%.9f\n", energy())
}
