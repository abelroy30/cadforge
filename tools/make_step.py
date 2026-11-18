#!/usr/bin/env python3
"""Generate STEP (ISO 10303-21) test parts for cadforge.

A small, dependency-free generator so we always have a real part to point the
studio at. It writes an AP214-flavoured file with a manifold solid B-rep.

What is exact and what is simplified:

* The box itself is exact: 8 vertices, 12 edges, 6 planar faces with correct
  loops, shared edge curves, and a closed shell. It is watertight.
* Holes get their real surfaces and curves - a CYLINDRICAL_SURFACE plus the two
  CIRCLE curves that bound it - but the top and bottom faces are written as
  plain rectangles and do not carry the hole loops. So a hole-aware kernel
  would read the hole faces and still see a closed outer shell, but the faces
  are not a fully trimmed solid. That is deliberate: it keeps the generator
  short and the geometry we want to read is all present.

Usage:
    tools/make_step.py plate > plate.step
    tools/make_step.py box   > box.step
"""

import sys


class StepFile:
    def __init__(self):
        self.body = []
        self.n = 0

    def add(self, text):
        self.n += 1
        self.body.append("#%d=%s;" % (self.n, text))
        return self.n

    def render(self, name, description):
        head = [
            "ISO-10303-21;",
            "HEADER;",
            "FILE_DESCRIPTION(('%s'),'2;1');" % description,
            "FILE_NAME('%s','2026-11-18T00:00:00',('cadforge'),('cadforge'),"
            "'cadforge tools/make_step.py','cadforge','');" % name,
            "FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));",
            "ENDSEC;",
            "DATA;",
        ]
        tail = ["ENDSEC;", "END-ISO-10303-21;", ""]
        return "\n".join(head + self.body + tail)


def num(v):
    s = ("%.6f" % v).rstrip("0").rstrip(".")
    return s + "." if "." not in s else s


def point(f, x, y, z):
    return f.add("CARTESIAN_POINT('',(%s,%s,%s))" % (num(x), num(y), num(z)))


def direction(f, x, y, z):
    return f.add("DIRECTION('',(%s,%s,%s))" % (num(x), num(y), num(z)))


def placement(f, origin, axis, ref):
    o = point(f, *origin)
    a = direction(f, *axis)
    r = direction(f, *ref)
    return f.add("AXIS2_PLACEMENT_3D('',#%d,#%d,#%d)" % (o, a, r))


def vertex(f, p):
    return f.add("VERTEX_POINT('',#%d)" % point(f, *p))


def vector(f, d, magnitude):
    dr = direction(f, *d)
    return f.add("VECTOR('',#%d,%s)" % (dr, num(magnitude)))


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def dist(a, b):
    d = sub(a, b)
    return (d[0] ** 2 + d[1] ** 2 + d[2] ** 2) ** 0.5


def unit(d):
    m = (d[0] ** 2 + d[1] ** 2 + d[2] ** 2) ** 0.5 or 1.0
    return (d[0] / m, d[1] / m, d[2] / m)


def edge(f, a, b):
    """An EDGE_CURVE from a to b, with a LINE supporting curve."""
    d = unit(sub(b, a))
    length = dist(a, b)
    ref = [1.0, 0.0, 0.0]
    if abs(d[0]) > 0.9:
        ref = [0.0, 1.0, 0.0]
    pl = placement(f, a, d, ref)
    ln = f.add("LINE('',#%d,#%d)" % (pl, vector(f, d, length)))
    va = vertex(f, a)
    vb = vertex(f, b)
    return f.add("EDGE_CURVE('',#%d,#%d,#%d,.T.)" % (va, vb, ln))


def face(f, loop_points, normal, ref):
    """A planar ADVANCED_FACE over an ordered vertex loop."""
    n = len(loop_points)
    oriented = []
    for i in range(n):
        a = loop_points[i]
        b = loop_points[(i + 1) % n]
        e = edge(f, a, b)
        oriented.append(f.add("ORIENTED_EDGE('',*,*,#%d,.T.)" % e))
    loop = f.add("EDGE_LOOP('',(%s))" % ",".join("#%d" % o for o in oriented))
    bound = f.add("FACE_OUTER_BOUND('',#%d,.T.)" % loop)
    pl = placement(f, loop_points[0], normal, ref)
    surf = f.add("PLANE('',#%d)" % pl)
    return f.add("ADVANCED_FACE('',(#%d),#%d,.T.)" % (bound, surf))


def hole(f, cx, cy, z0, z1, radius):
    """A cylindrical face with its two bounding circles."""
    axis = (0.0, 0.0, 1.0)
    ref = (1.0, 0.0, 0.0)
    bottom_pl = placement(f, (cx, cy, z0), axis, ref)
    top_pl = placement(f, (cx, cy, z1), axis, ref)
    cyl = f.add("CYLINDRICAL_SURFACE('',#%d,%s)" % (bottom_pl, num(radius)))
    c0 = f.add("CIRCLE('',#%d,%s)" % (bottom_pl, num(radius)))
    c1 = f.add("CIRCLE('',#%d,%s)" % (top_pl, num(radius)))
    v0a = vertex(f, (cx + radius, cy, z0))
    v0b = vertex(f, (cx - radius, cy, z0))
    v1a = vertex(f, (cx + radius, cy, z1))
    v1b = vertex(f, (cx - radius, cy, z1))
    e0 = f.add("EDGE_CURVE('',#%d,#%d,#%d,.T.)" % (v0a, v0b, c0))
    e1 = f.add("EDGE_CURVE('',#%d,#%d,#%d,.T.)" % (v1a, v1b, c1))
    o0 = f.add("ORIENTED_EDGE('',*,*,#%d,.T.)" % e0)
    o1 = f.add("ORIENTED_EDGE('',*,*,#%d,.F.)" % e1)
    loop = f.add("EDGE_LOOP('',(#%d,#%d))" % (o0, o1))
    bound = f.add("FACE_OUTER_BOUND('',#%d,.T.)" % loop)
    return f.add("ADVANCED_FACE('',(#%d),#%d,.T.)" % (bound, cyl))


def build_box_holes(width, depth, height, holes, radius):
    f = StepFile()
    names = {
        (0, 0, 0): (0.0, 0.0, 0.0), (1, 0, 0): (width, 0.0, 0.0),
        (1, 1, 0): (width, depth, 0.0), (0, 1, 0): (0.0, depth, 0.0),
        (0, 0, 1): (0.0, 0.0, height), (1, 0, 1): (width, 0.0, height),
        (1, 1, 1): (width, depth, height), (0, 1, 1): (0.0, depth, height),
    }
    faces = []
    faces.append(face(f, [names[(0, 0, 0)], names[(1, 0, 0)], names[(1, 1, 0)], names[(0, 1, 0)]], (0, 0, -1), (1, 0, 0)))
    faces.append(face(f, [names[(0, 0, 1)], names[(0, 1, 1)], names[(1, 1, 1)], names[(1, 0, 1)]], (0, 0, 1), (1, 0, 0)))
    faces.append(face(f, [names[(0, 0, 0)], names[(0, 0, 1)], names[(1, 0, 1)], names[(1, 0, 0)]], (0, -1, 0), (1, 0, 0)))
    faces.append(face(f, [names[(0, 1, 0)], names[(1, 1, 0)], names[(1, 1, 1)], names[(0, 1, 1)]], (0, 1, 0), (1, 0, 0)))
    faces.append(face(f, [names[(0, 0, 0)], names[(0, 1, 0)], names[(0, 1, 1)], names[(0, 0, 1)]], (-1, 0, 0), (0, 1, 0)))
    faces.append(face(f, [names[(1, 0, 0)], names[(1, 0, 1)], names[(1, 1, 1)], names[(1, 1, 0)]], (1, 0, 0), (0, 1, 0)))
    for (cx, cy) in holes:
        faces.append(hole(f, cx, cy, 0.0, height, radius))
    shell = f.add("CLOSED_SHELL('',(%s))" % ",".join("#%d" % x for x in faces))
    solid = f.add("MANIFOLD_SOLID_BREP('part',#%d)" % shell)
    app = f.add("APPLICATION_CONTEXT('automotive design')")
    mech = f.add("MECHANICAL_CONTEXT('',#%d,'mechanical')" % app)
    product = f.add("PRODUCT('cadforge_test','cadforge test part','',(#%d))" % mech)
    pdf = f.add("PRODUCT_DEFINITION_FORMATION('','',#%d)" % product)
    pd = f.add("PRODUCT_DEFINITION('design','',#%d,#%d)" % (pdf, app))
    shape = f.add("PRODUCT_DEFINITION_SHAPE('','',#%d)" % pd)
    unit = f.add("SI_UNIT(.MILLI.,.METRE.)")
    unc = f.add("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-6),#%d,'','')" % unit)
    len_unit = f.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))")
    ctx = f.add("(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#%d))GLOBAL_UNIT_ASSIGNED_CONTEXT((#%d))REPRESENTATION_CONTEXT('','3D'))" % (unc, len_unit))
    rep = f.add("ADVANCED_BREP_SHAPE_REPRESENTATION('part',(#%d),#%d)" % (solid, ctx))
    f.add("SHAPE_DEFINITION_REPRESENTATION(#%d,#%d)" % (shape, rep))
    return f


def main(argv):
    part = (argv[0] if argv else "plate").lower()
    if part == "box":
        f = build_box_holes(60.0, 40.0, 8.0, [], 0.0)
        out = f.render("box.step", "plain box, no features")
    elif part == "plate":
        holes = [(12.0, 10.0), (48.0, 10.0), (12.0, 30.0), (48.0, 30.0)]
        f = build_box_holes(60.0, 40.0, 8.0, holes, 3.0)
        out = f.render("plate.step", "60x40x8 plate with 4 holes D6")
    else:
        sys.stderr.write("unknown part %r (try: box, plate)\n" % part)
        return 2
    sys.stdout.write(out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
