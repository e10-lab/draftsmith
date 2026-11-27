"""시험 도면 ezdxf-kinds.dxf 만들기 (구현 노트 §88): 실무에서 가끔 나오는 객체 종류를 하나씩.
쓰는 법: python make_ezdxf_kinds.py ezdxf-kinds.dxf (ezdxf 필요)"""
import sys, ezdxf
from ezdxf.math import Vec3, Vec2
doc = ezdxf.new("R2018", setup=True)
m = doc.modelspace()
m.add_line((0,0),(100,0))
m.add_point((10,10))
m.add_solid([(0,20),(20,20),(0,40),(20,40)])
m.add_3dface([(30,20,0),(50,20,0),(50,40,0),(30,40,0)])
m.add_xline((0,60),(1,1))
m.add_ray((0,70),(1,0))
m.add_spline(fit_points=[(0,80),(20,90),(40,80),(60,95)])
m.add_polyline2d([(0,100),(20,110),(40,100)])
m.add_mtext("여러 줄\\P문자", dxfattribs={"insert": (0,120), "char_height": 5})
ld = m.add_leader([(0,140),(20,150),(40,150)])
ml = m.add_multileader_mtext("Standard")
ml.set_content("지시선 글", char_height=3)
ml.add_leader_line(ezdxf.render.mleader.ConnectionSide.left, [Vec2(60,140)])
ml.build(insert=Vec2(80,150))
try:
    mline = m.add_mline([(0,170),(50,170),(50,200)])
except Exception as e:
    print("mline 실패", e)

w = m.add_wipeout([(100,0),(120,0),(120,20),(100,20)])
img_def = doc.add_image_def(filename="x.png", size_in_pixel=(64,64))
m.add_image(img_def, insert=(100,40), size_in_units=(20,20))
m.add_circle((150,150), 10)
doc.saveas(sys.argv[1])
print("ok")
