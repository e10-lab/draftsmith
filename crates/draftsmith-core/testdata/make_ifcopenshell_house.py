# IfcOpenShell 0.9 로 ifcopenshell-house.ifc 를 만드는 스크립트 (IFC 가져오기 시험 자료, 구현 노트 §47)
# 쓰는 법: python make_ifcopenshell_house.py ifcopenshell-house.ifc
import sys, numpy as np, ifcopenshell, ifcopenshell.api as api
f = api.run("project.create_file", version="IFC4")
proj = api.run("root.create_entity", f, ifc_class="IfcProject", name="외부")
api.run("unit.assign_unit", f)  # 기본: mm
ctx = api.run("context.add_context", f, context_type="Model")
body = api.run("context.add_context", f, context_type="Model", context_identifier="Body", target_view="MODEL_VIEW", parent=ctx)
axisctx = api.run("context.add_context", f, context_type="Plan", context_identifier="Axis", target_view="GRAPH_VIEW", parent=ctx)
site = api.run("root.create_entity", f, ifc_class="IfcSite")
bld = api.run("root.create_entity", f, ifc_class="IfcBuilding")
api.run("aggregate.assign_object", f, relating_object=proj, products=[site])
api.run("aggregate.assign_object", f, relating_object=site, products=[bld])
st = []
for i, z in enumerate([0, 3200]):
    s = api.run("root.create_entity", f, ifc_class="IfcBuildingStorey", name=f"{i+1}F")
    s.Elevation = z
    api.run("geometry.edit_object_placement", f, product=s, matrix=np.array([[1,0,0,0],[0,1,0,0],[0,0,1,z/1000],[0,0,0,1]],float))
    api.run("aggregate.assign_object", f, relating_object=bld, products=[s])
    st.append(s)
def wall(s, x, y, ang, L, z):
    w = api.run("root.create_entity", f, ifc_class="IfcWall", name="W")
    c, si = np.cos(ang), np.sin(ang)
    api.run("geometry.edit_object_placement", f, product=w, matrix=np.array([[c,-si,0,x/1000],[si,c,0,y/1000],[0,0,1,z/1000],[0,0,0,1]],float))
    rep = api.run("geometry.add_wall_representation", f, context=body, length=L/1000, height=3.0, thickness=0.2)
    api.run("geometry.assign_representation", f, product=w, representation=rep)
    api.run("spatial.assign_container", f, relating_structure=s, products=[w])
    return w
w1 = wall(st[0], 0, 0, 0, 6000, 0)
w2 = wall(st[0], 6000, 0, np.pi/2, 4000, 0)
w3 = wall(st[1], 0, 0, 0, 5000, 3200)
# 문: w1 의 x=2000~2900
op = api.run("root.create_entity", f, ifc_class="IfcOpeningElement")
api.run("geometry.edit_object_placement", f, product=op, matrix=np.array([[1,0,0,2],[0,1,0,-0.05],[0,0,1,0],[0,0,0,1]],float))
orep = api.run("geometry.add_wall_representation", f, context=body, length=0.9, height=2.1, thickness=0.3)
api.run("geometry.assign_representation", f, product=op, representation=orep)
api.run("feature.add_feature", f, feature=op, element=w1)
d = api.run("root.create_entity", f, ifc_class="IfcDoor", name="D")
d.OverallWidth = 900; d.OverallHeight = 2100
api.run("geometry.edit_object_placement", f, product=d, matrix=np.array([[1,0,0,2],[0,1,0,0],[0,0,1,0],[0,0,0,1]],float))
api.run("feature.add_filling", f, opening=op, element=d)
api.run("spatial.assign_container", f, relating_structure=st[0], products=[d])
# 실
sp = api.run("root.create_entity", f, ifc_class="IfcSpace", name="101", )
sp.LongName = "거실"
api.run("geometry.edit_object_placement", f, product=sp, matrix=np.eye(4))
srep = api.run("geometry.add_slab_representation", f, context=body, depth=3.0, polyline=[(0,0),(6,0),(6,4),(0,4)])
api.run("geometry.assign_representation", f, product=sp, representation=srep)
api.run("aggregate.assign_object", f, relating_object=st[0], products=[sp])
# 원기둥
col = api.run("root.create_entity", f, ifc_class="IfcColumn")
api.run("geometry.edit_object_placement", f, product=col, matrix=np.array([[1,0,0,3],[0,1,0,2],[0,0,1,0],[0,0,0,1]],float))
prof = f.create_entity("IfcCircleProfileDef", ProfileType="AREA", Radius=250.0)
crep = api.run("geometry.add_profile_representation", f, context=body, profile=prof, depth=3.0)
api.run("geometry.assign_representation", f, product=col, representation=crep)
api.run("spatial.assign_container", f, relating_structure=st[0], products=[col])
f.write(sys.argv[1])
