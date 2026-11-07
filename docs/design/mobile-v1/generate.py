"""Generate one editable layer document, Figma plugin, SVG screens and PNG previews.
No dependencies beyond Pillow (only used for local PNG previews).
"""
from pathlib import Path
from html import escape
import json, math, copy
from PIL import Image, ImageDraw, ImageFont
ROOT=Path(__file__).parent
BG='#0E1119'; SURFACE='#191F2C'; RAISED='#242D40'; BORDER='#303A50'; TEXT='#F2F4FA'; MUTED='#9EAAC0'; ACCENT='#B9C8FF'; MINT='#78DFC0'; RED='#F49BA8'
SCREENS=[]
def rect(a,x,y,w,h,fill=SURFACE,r=16,stroke=None):
 a.append(dict(type='rect',x=x,y=y,w=w,h=h,fill=fill,r=r,stroke=stroke))
def text(a,x,y,s,size=14,color=TEXT,weight=400):
 a.append(dict(type='text',x=x,y=y,text=s,size=size,color=color,weight=weight))
def circle(a,x,y,r,fill): a.append(dict(type='ellipse',x=x-r,y=y-r,w=2*r,h=2*r,fill=fill))
def line(a,x,y,x2,y2,color=MUTED,width=1.6): a.append(dict(type='line',x=x,y=y,x2=x2,y2=y2,color=color,width=width))
def icon(a,name,x,y,color=TEXT,s=22):
 def L(p,q): line(a,x+p[0]*s/24,y+p[1]*s/24,x+q[0]*s/24,y+q[1]*s/24,color,1.7)
 def R(xx,yy,w,h,r=2): rect(a,x+xx*s/24,y+yy*s/24,w*s/24,h*s/24,BG,r,color)
 if name in ['monitor','laptop']:
  R(3,4,18,12); L((8,20),(16,20)); L((12,16),(12,20))
 elif name=='mic':
  R(9,2,6,13,3); L((6,10),(6,13)); L((6,13),(9,18)); L((9,18),(15,18)); L((15,18),(18,13)); L((18,13),(18,10)); L((12,18),(12,22)); L((8,22),(16,22))
 elif name=='mic-off': icon(a,'mic',x,y,color,s);L((3,3),(21,21))
 elif name=='back': L((15,5),(8,12));L((8,12),(15,19))
 elif name in ['arrow','chevron']: L((9,5),(16,12));L((16,12),(9,19))
 elif name=='close':L((6,6),(18,18));L((18,6),(6,18))
 elif name=='plus':L((12,5),(12,19));L((5,12),(19,12))
 elif name=='search':
  a.append(dict(type='ellipse',x=x+3,y=y+3,w=s*.55,h=s*.55,fill=BG,stroke=color));L((16,16),(21,21))
 elif name=='check':L((5,12),(10,17));L((10,17),(20,6))
 elif name=='speaker':
  L((4,9),(8,9));L((8,9),(13,4));L((13,4),(13,20));L((13,20),(8,15));L((8,15),(4,15));L((4,15),(4,9));L((17,8),(20,12));L((20,12),(17,16))
 elif name=='keyboard':
  R(2,5,20,14)
  for yy in [9,12]:
   for xx in [6,10,14,18]:L((xx,yy),(xx+.8,yy))
  L((7,16),(17,16))
 elif name=='touch':
  R(5,2,14,20,4);L((10,17),(14,17));circle(a,x+s/2,y+s*.42,2,color)
 elif name=='shield':
  for p,q in [((12,2),(20,5)),((20,5),(19,15)),((19,15),(12,22)),((12,22),(5,15)),((5,15),(4,5)),((4,5),(12,2))]:L(p,q)
 elif name=='users':
  for xx in [8,17]:
   a.append(dict(type='ellipse',x=x+(xx-3)*s/24,y=y+3*s/24,w=s/4,h=s/4,fill=BG,stroke=color))
  L((2,20),(3,15));L((3,15),(8,13));L((8,13),(13,15));L((13,15),(14,20));L((17,13),(21,16));L((21,16),(22,20))
 elif name=='person':
  a.append(dict(type='ellipse',x=x+7*s/24,y=y+2*s/24,w=s*10/24,h=s*10/24,fill=BG,stroke=color));L((3,22),(4,17));L((4,17),(12,14));L((12,14),(20,17));L((20,17),(21,22))
 elif name=='wifi':
  L((2,8),(7,5));L((7,5),(17,5));L((17,5),(22,8));L((6,12),(9,10));L((9,10),(15,10));L((15,10),(18,12));circle(a,x+s/2,y+s*.75,2,color)
 elif name=='expand':
  for p,q in [((3,9),(3,3)),((3,3),(9,3)),((15,3),(21,3)),((21,3),(21,9)),((21,15),(21,21)),((21,21),(15,21)),((9,21),(3,21)),((3,21),(3,15))]:L(p,q)
 elif name=='leave':
  L((10,4),(4,4));L((4,4),(4,20));L((4,20),(10,20));L((10,12),(21,12));L((17,8),(21,12));L((21,12),(17,16))
 elif name=='dots':
  for xx in [5,12,19]:circle(a,x+xx*s/24,y+s/2,1.4,color)
 elif name=='lock':R(5,10,14,11);R(8,3,8,8,4)
 elif name=='settings':
  R(4,4,16,16,5);a.append(dict(type='ellipse',x=x+8*s/24,y=y+8*s/24,w=s/3,h=s/3,fill=BG,stroke=color))

def pill(a,x,y,s,color=MINT,w=None):
 w=w or len(s)*6+26;rect(a,x,y,w,25,'#203D37' if color==MINT else RAISED,12);circle(a,x+11,y+12.5,2.5,color);text(a,x+19,y+5,s,10,color,600)
def button(a,x,y,w,label,kind='primary',ico=None):
 fill=ACCENT if kind=='primary' else ('#34232E' if kind=='danger' else RAISED); color=BG if kind=='primary' else (RED if kind=='danger' else TEXT)
 rect(a,x,y,w,50,fill,15)
 if ico:icon(a,ico,x+16,y+14,color,22);text(a,x+49,y+16,label,14,color,600)
 else:text(a,x+max(14,(w-len(label)*7.2)/2),y+16,label,14,color,600)
def status(a,w,android=False):
 text(a,25,16,'9:41',13,TEXT,600)
 if not android:rect(a,w/2-45,10,90,25,'#06080D',16)
 for i in range(4):rect(a,w-76+i*4,25-i*2,2,3+i*2,TEXT,0)
 icon(a,'wifi',w-57,17,TEXT,15);rect(a,w-33,21,19,9,TEXT,3);rect(a,w-12,24,2,3,TEXT,0)
def bottom(a,w,h,active=0,android=False):
 rect(a,0,h-95,w,95,'#141A25',0);line(a,0,h-95,w,h-95,BORDER,1)
 for i,(ic,label) in enumerate([('monitor','Máy tính'),('users','Chia sẻ'),('person','Tài khoản')]):
  x=w*(i+.5)/3
  if i==active:rect(a,x-27,h-87,54,33,'#2B3550',17)
  icon(a,ic,x-11,h-82,ACCENT if i==active else MUTED,22);text(a,x-len(label)*2.6,h-47,label,10,ACCENT if i==active else MUTED,600 if i==active else 400)
 rect(a,w/2-55,h-10,110,4,'#C6CCD9',3)
def header(a,title,sub=None,w=390,back=False):
 if back:rect(a,20,61,44,44,SURFACE,14);icon(a,'back',31,72,TEXT,22);tx=78
 else:tx=24
 text(a,tx,69,title,24,TEXT,700)
 if sub:text(a,tx,104,sub,12,MUTED)
def add(id,name,w=390,h=844,android=False):
 a=[];rect(a,0,0,w,h,BG,0);status(a,w,android)
 d=dict(id=id,name=name,width=w,height=h,nodes=a,hotspots=[]);SCREENS.append(d);return d,a
def hot(d,x,y,w,h,to,label):d['hotspots'].append(dict(x=x,y=y,w=w,h=h,to=to,label=label))
def desktop(a,x,y,w,h,mac=False):
 a.append(dict(type='clip-start',x=x,y=y,w=w,h=h,r=12))
 rect(a,x,y,w,h,'#283B50',12)
 # Geometric wallpaper, built from editable vector shapes.
 circle(a,x+w*.75,y+h*.43,h*.5,'#3E6471');circle(a,x+w*.78,y+h*.73,h*.38,'#638C88');circle(a,x+w*.22,y+h*.82,h*.58,'#283548')
 rect(a,x+14,y+12,w-28,22,'#182230',6);text(a,x+24,y+18,'Finder' if mac else 'Sanser Workspace',8,TEXT,600)
 rect(a,x+w*.11,y+h*.24,w*.58,h*.55,'#131C29',8)
 for k,(label,co) in enumerate([('Project Sanser',ACCENT),('Design / Mobile',TEXT),('Ready when you are.',MUTED)]):text(a,x+w*.15,y+h*.29+k*22,label,12 if k==0 else 9,co,600 if k==0 else 400)
 rect(a,x+w*.71,y+h*.29,w*.19,h*.36,'#9BC4B6',8)
 for k in range(3):rect(a,x+w*.74,y+h*.35+k*13,w*.11,4,'#446E67',2)
 rect(a,x+w*.28,y+h-21,w*.44,12,'#1F2A3C',4)
 a.append(dict(type='clip-end',x=0,y=0))

def home(android=False):
 w=412 if android else 390;h=892 if android else 844
 d,a=add('android-home' if android else 'home','Android / Máy tính' if android else 'iOS / Máy tính',w,h,android)
 rect(a,24,64,38,38,'#1D2538',12);text(a,33,67,'S',27,ACCENT,700);circle(a,55,71,3,MINT);text(a,73,65,'Sanser',23,TEXT,700);text(a,74,94,'Không gian của bạn, ở mọi nơi.',10,MUTED)
 circle(a,w-42,84,20,RAISED);text(a,w-49,77,'D',14,ACCENT,600)
 text(a,24,145,'Máy tính của bạn',26,TEXT,700);text(a,24,182,'2 máy trực tuyến · Sẵn sàng kết nối',12,MUTED)
 rect(a,24,214,w-48,48,SURFACE,14,BORDER);icon(a,'search',40,228,MUTED,20);text(a,72,230,'Tìm tên máy hoặc mã thiết bị',12,MUTED)
 for xx,ww,label,active in [(24,88,'Tất cả',True),(121,104,'Trực tuyến',False),(234,90,'Đã ghim',False)]:rect(a,xx,278,ww,32,RAISED if active else BG,16);text(a,xx+16,287,label,11,ACCENT if active else MUTED,600)
 yy=330;cw=w-48
 rect(a,24,yy,cw,192,SURFACE,21,BORDER);rect(a,40,yy+16,52,48,'#28334C',14);icon(a,'monitor',54,yy+28,ACCENT,24);text(a,105,yy+18,'Windows Studio',17,TEXT,600);text(a,105,yy+44,'Windows 11 · Phòng làm việc',10,MUTED);pill(a,w-121,yy+77,'Trực tuyến',MINT,81)
 text(a,42,yy+82,'Thiết bị đã tin cậy',11,MUTED);button(a,40,yy+121,cw-32,'Kết nối',ico='arrow');hot(d,24,yy,cw,192,'device','Mở Windows Studio')
 rect(a,24,538,cw,91,SURFACE,19,BORDER);rect(a,40,555,48,48,'#293D40',14);icon(a,'laptop',52,567,MINT,24);text(a,102,558,'MacBook Pro',16,TEXT,600);text(a,102,584,'macOS · Trực tuyến',11,MUTED);icon(a,'chevron',w-61,571,MUTED,20)
 rect(a,24,648,cw,64,'#232A3F',16);icon(a,'users',40,668,ACCENT,22);text(a,74,661,'Lời mời chia sẻ mới',13,TEXT,600);text(a,74,682,'Linh đang chia sẻ màn hình',10,MUTED);icon(a,'arrow',w-56,668,ACCENT,20);hot(d,24,648,cw,64,'sharing','Xem lời mời chia sẻ')
 bottom(a,w,h,0,android)
 hot(d,0,h-95,w/3,76,d['id'],'Máy tính');hot(d,w/3,h-95,w/3,76,'sharing','Chia sẻ');hot(d,w*2/3,h-95,w/3,76,'settings','Tài khoản')
 return d
home();home(True)
d,a=add('device','Chi tiết / Quyền truy cập');header(a,'Windows Studio','Máy tính đã tin cậy',back=True);hot(d,20,61,44,44,'home','Quay lại')
desktop(a,24,145,342,198);pill(a,40,158,'Trực tuyến',MINT,82)
text(a,24,371,'Bạn muốn kết nối thế nào?',20,TEXT,600)
rect(a,24,414,342,83,'#252F48',18,ACCENT);icon(a,'touch',42,442,ACCENT,25);text(a,86,431,'Điều khiển máy tính',15,TEXT,600);text(a,86,459,'Dùng chuột, bàn phím và cảm ứng',10,MUTED);circle(a,340,454,9,ACCENT);circle(a,340,454,4,BG)
rect(a,24,509,342,83,SURFACE,18,BORDER);icon(a,'monitor',42,538,MUTED,24);text(a,86,527,'Chỉ xem màn hình',15,TEXT,600);text(a,86,555,'Không gửi thao tác đến máy tính',10,MUTED);hot(d,24,509,342,83,'sharing','Xem thử chế độ chỉ xem')
icon(a,'shield',27,620,MINT,20);text(a,57,619,'Mã hóa đầu cuối',12,TEXT,600);text(a,57,642,'Máy chủ cần cho phép truy cập trước khi mở.',10,MUTED)
button(a,24,715,342,'Kết nối an toàn',ico='arrow');hot(d,24,715,342,50,'connecting','Bắt đầu kết nối');text(a,67,786,'Chỉ số kết nối chỉ xuất hiện sau khi đo.',10,MUTED)
view=copy.deepcopy(d);view['id']='device-view';view['name']='Chi tiết / Chỉ xem';SCREENS.append(view)
for n in view['nodes']:
 if n['type']=='rect' and n['x']==24 and n['y']==414:n['fill']=SURFACE;n['stroke']=BORDER
 if n['type']=='rect' and n['x']==24 and n['y']==509:n['fill']='#252F48';n['stroke']=ACCENT
 if n['type']=='ellipse' and n['x']==331 and n['y']==445:n['fill']=BORDER
circle(view['nodes'],340,549,9,ACCENT);circle(view['nodes'],340,549,4,BG)
for h in d['hotspots']:
 if h['to']=='sharing':h['to']='device-view';h['label']='Chọn chế độ chỉ xem'
view['hotspots']=[dict(x=20,y=61,w=44,h=44,to='home',label='Quay lại'),dict(x=24,y=414,w=342,h=83,to='device',label='Chọn điều khiển'),dict(x=24,y=715,w=342,h=50,to='connecting-view',label='Kết nối chỉ xem')]
d,a=add('connecting','Kết nối / Chờ hình đầu tiên');header(a,'Đang kết nối',back=True);hot(d,20,61,44,44,'device','Hủy kết nối')
circle(a,195,247,65,'#1C273C');icon(a,'monitor',165,215,ACCENT,60);circle(a,240,289,18,MINT);icon(a,'check',229,278,BG,22)
text(a,71,345,'Đang mở Windows Studio',20,TEXT,600);text(a,64,381,'Chờ máy tính gửi khung hình đầu tiên.',13,MUTED)
for i,(label,sub) in enumerate([('Đã xác thực','Thiết bị của bạn'),('Đã tìm thấy đường kết nối','Đang kiểm tra luồng hình ảnh'),('Chờ hình ảnh','Chưa bắt đầu điều khiển')]):
 yy=444+i*78;circle(a,42,yy+15,14,MINT if i<2 else RAISED);icon(a,'check',33,yy+6,BG,18) if i<2 else text(a,38,yy+7,'3',12,MUTED,600);text(a,70,yy+2,label,14,TEXT,600);text(a,70,yy+28,sub,11,MUTED)
button(a,24,709,342,'Xem phiên mô phỏng',ico='expand');hot(d,24,709,342,50,'remote','Mở điều khiển mô phỏng');text(a,58,782,'Bản thiết kế · Không kết nối đến máy thật',11,MUTED);hot(d,24,615,342,68,'failure','Xem trạng thái chờ quá lâu')
d,a=add('sharing','Chia sẻ / Lời mời');header(a,'Chia sẻ','Xem cùng nhau. Trò chuyện cùng nhau.');bottom(a,390,844,1)
rect(a,24,153,342,356,SURFACE,22,BORDER);pill(a,41,169,'Lời mời mới',ACCENT,104);desktop(a,40,211,310,170,True);text(a,42,399,'Màn hình của Linh',20,TEXT,600);text(a,42,434,'MacBook Pro · 2 người đang xem',12,MUTED);icon(a,'lock',42,467,MUTED,17);text(a,69,469,'Chỉ xem · Mic luôn tắt khi mới tham gia',10,MUTED)
button(a,24,532,342,'Tham gia chia sẻ',ico='users');hot(d,24,532,342,50,'viewer','Tham gia xem mô phỏng')
text(a,24,625,'Voice theo phiên',16,TEXT,600);text(a,24,656,'Bật mic khi bạn sẵn sàng. Có thể tắt tiếng',12,MUTED);text(a,24,676,'của mình hoặc tắt âm cuộc trò chuyện riêng.',12,MUTED)
hot(d,0,749,130,76,'home','Máy tính');hot(d,260,749,130,76,'settings','Tài khoản')
d,a=add('voice','Voice / Mic tắt mặc định');header(a,'Đang xem chia sẻ','Màn hình của Linh',back=True);desktop(a,24,152,342,212,True);pill(a,40,167,'Chỉ xem',ACCENT,82)
rect(a,16,410,358,420,'#1A2231',28,BORDER);rect(a,171,424,48,4,'#536078',2);text(a,40,451,'Trò chuyện trong phiên',21,TEXT,700);text(a,40,487,'3 người · Âm thanh loa đang bật',12,MUTED);icon(a,'close',326,452,MUTED,24);hot(d,313,441,46,46,'viewer','Thu gọn voice')
for i,(name,sub,co,ic) in enumerate([('Linh','Đang nói',MINT,'mic'),('Minh','Đã tắt mic',MUTED,'mic-off'),('Bạn','Mic đang tắt',ACCENT,'mic-off')]):
 yy=532+i*57;circle(a,59,yy+19,18,'#2D3C50');text(a,53,yy+10,name[0],14,co,600);text(a,88,yy+1,name,14,TEXT,600);text(a,88,yy+24,sub,10,co);icon(a,ic,324,yy+8,co,21)
 if i==0:
  for k,hh in enumerate([6,13,19,10]):rect(a,267+k*6,yy+20-hh/2,3,hh,MINT,2)
button(a,36,723,156,'Bật mic',ico='mic');button(a,202,723,148,'Tắt loa',kind='secondary',ico='speaker');hot(d,36,723,156,50,'voice-live','Bật mic mô phỏng');hot(d,202,723,148,50,'voice-deaf','Tắt loa voice mô phỏng');text(a,73,792,'Bạn tự quyết định khi nào bắt đầu nói.',10,MUTED)
# Variants are independent editable frames, linked by preview hotspots.
import copy
live=copy.deepcopy(d);live['id']='voice-live';live['name']='Voice / Đang bật mic';SCREENS.append(live)
for n in live['nodes']:
 if n.get('text')=='Mic đang tắt':n['text']='Mic đang bật';n['color']=MINT
 if n.get('text')=='Bật mic':n['text']='Tắt mic'
 if n.get('text')=='Bạn tự quyết định khi nào bắt đầu nói.':n['text']='Bạn đang bật mic · Chỉ là mô phỏng UI'
live['nodes']=[n for n in live['nodes'] if not (n['type']=='line' and abs(n['x']-326.625)<.01 and abs(n['y']-656.625)<.01 and abs(n['x2']-342.375)<.01)]
for n in live['nodes']:
 if n['type']=='rect' and n['x']==36 and n['y']==723:n['fill']=MINT
 if 324<=n['x']<=345 and 654<=n['y']<=677:
  if n.get('color'):n['color']=MINT
  if n.get('stroke'):n['stroke']=MINT
live['hotspots']=[dict(x=36,y=723,w=156,h=50,to='voice',label='Tắt mic mô phỏng'),dict(x=313,y=441,w=46,h=46,to='viewer',label='Thu gọn voice')]
deaf=copy.deepcopy(d);deaf['id']='voice-deaf';deaf['name']='Voice / Tắt loa';SCREENS.append(deaf)
for n in deaf['nodes']:
 if n.get('text')=='3 người · Âm thanh loa đang bật':n['text']='Đã tắt loa voice · Mic vẫn tắt'
 if n.get('text')=='Tắt loa':n['text']='Bật loa'
deaf['hotspots']=[dict(x=202,y=723,w=148,h=50,to='voice',label='Bật loa voice mô phỏng'),dict(x=313,y=441,w=46,h=46,to='viewer',label='Thu gọn voice')]

live_deaf=copy.deepcopy(live);live_deaf['id']='voice-live-deaf';live_deaf['name']='Voice / Mic bật, loa tắt';SCREENS.append(live_deaf)
for n in live_deaf['nodes']:
 if n.get('text')=='3 người · Âm thanh loa đang bật':n['text']='Loa voice tắt · Mic đang bật'
 if n.get('text')=='Tắt loa':n['text']='Bật loa'
live['hotspots'].append(dict(x=202,y=723,w=148,h=50,to='voice-live-deaf',label='Tắt loa, giữ mic bật'))
deaf['hotspots'].append(dict(x=36,y=723,w=156,h=50,to='voice-live-deaf',label='Bật mic, giữ loa tắt'))
live_deaf['hotspots']=[dict(x=36,y=723,w=156,h=50,to='voice-deaf',label='Tắt mic, giữ loa tắt'),dict(x=202,y=723,w=148,h=50,to='voice-live',label='Bật loa, giữ mic bật'),dict(x=313,y=441,w=46,h=46,to='viewer',label='Thu gọn voice')]
def landscape(viewer=False):
 id='viewer' if viewer else 'remote';d,a=add(id,'Xem chia sẻ / Ngang' if viewer else 'Điều khiển / Ngang',844,390)
 # Remove portrait status bar; remote canvas gets inset safe controls.
 a.clear();rect(a,0,0,844,390,BG,0);desktop(a,34,10,776,348,viewer)
 rect(a,55,28,242,44,'#141C29',16);icon(a,'back',68,39,TEXT,22);text(a,105,37,'Màn hình của Linh' if viewer else 'Windows Studio',13,TEXT,600);text(a,105,55,'Chỉ xem' if viewer else 'Điều khiển cảm ứng',9,MINT)
 hot(d,55,28,44,44,'sharing' if viewer else 'home','Rời cửa sổ xem')
 rect(a,560,28,229,48,'#141C29',17,BORDER)
 for x,ic,to,label in [(574,'mic-off','voice','Mở voice'),(626,'speaker' if viewer else 'keyboard','voice' if viewer else 'keyboard','Âm thanh' if viewer else 'Bàn phím'),(678,'users' if viewer else 'touch','voice' if viewer else 'touch-guide','Người tham gia' if viewer else 'Thao tác cảm ứng'),(730,'leave','disconnect','Rời phiên')]:
  icon(a,ic,x,41,RED if ic=='leave' else ACCENT,22);hot(d,x-8,30,46,44,to,label)
 rect(a,195,308,454,38,'#141C29',13)
 text(a,213,320,'Chỉ xem · Nhấn mic để tham gia voice' if viewer else '1 chạm: nhấp · 2 ngón: cuộn · Giữ: chuột phải',11,TEXT)
 rect(a,228,359,388,21,'#18212F',10);text(a,246,365,'DEMO   ·   1080p / 60 FPS   ·   RTT 12 ms (minh họa)',9,MUTED)
 rect(a,367,382,110,4,'#C6CCD9',3)
 return d
landscape();landscape(True)
cv=copy.deepcopy(next(s for s in SCREENS if s['id']=='connecting'));cv['id']='connecting-view';cv['name']='Kết nối / Chỉ xem';SCREENS.append(cv)
for h in cv['hotspots']:
 if h['to']=='remote':h['to']='viewer-single'
 if h['to']=='device':h['to']='device-view'
vs=copy.deepcopy(next(s for s in SCREENS if s['id']=='viewer'));vs['id']='viewer-single';vs['name']='Chỉ xem / Windows Studio';SCREENS.append(vs)
for n in vs['nodes']:
 if n.get('text')=='Màn hình của Linh':n['text']='Windows Studio'
 if n.get('text')=='Finder':n['text']='Sanser Workspace'
 if n.get('text')=='Chỉ xem · Nhấn mic để tham gia voice':n['text']='Chỉ xem · Không gửi thao tác chuột hoặc bàn phím'
vs['hotspots']=[dict(x=55,y=28,w=44,h=44,to='device-view',label='Về chi tiết máy'),dict(x=722,y=30,w=46,h=44,to='disconnect',label='Rời phiên chỉ xem')]
d,a=add('touch-guide','Điều khiển / Hướng dẫn chạm');header(a,'Chạm để điều khiển','Các thao tác dùng trong phiên',back=True);hot(d,20,61,44,44,'remote','Về phiên điều khiển')
for i,(title,sub,ico) in enumerate([('Một chạm','Nhấp chuột trái','touch'),('Giữ một lúc','Mở menu chuột phải','touch'),('Hai ngón tay','Cuộn trang theo cả hai chiều','touch'),('Bàn phím','Mở bàn phím và các phím bổ trợ','keyboard')]):
 yy=161+i*113;rect(a,24,yy,342,96,SURFACE,19,BORDER);icon(a,ico,43,yy+32,ACCENT,28);text(a,96,yy+24,title,16,TEXT,600);text(a,96,yy+54,sub,11,MUTED)
button(a,24,691,342,'Đã hiểu, mở màn hình',ico='expand');hot(d,24,691,342,50,'remote','Về điều khiển')
d,a=add('keyboard','Điều khiển / Bàn phím');header(a,'Bàn phím từ xa','Giữ Ctrl / Shift để ghép tổ hợp',back=True);hot(d,20,61,44,44,'remote','Đóng bàn phím')
desktop(a,24,150,342,235);text(a,24,418,'Phím bổ trợ',15,TEXT,600)
for i,key in enumerate(['Ctrl','Alt','Shift','Win']):rect(a,24+i*88,455,78,48,RAISED,13);text(a,44+i*88,471,key,13,TEXT,600)
rect(a,24,533,342,127,SURFACE,18,BORDER);text(a,42,553,'Gõ để gửi tới máy tính…',15,MUTED);text(a,42,602,'Vùng bàn phím hệ thống sẽ mở tại đây.',11,MUTED)
button(a,24,704,342,'Quay về màn hình',ico='expand');hot(d,24,704,342,50,'remote','Đóng bàn phím mô phỏng')
d,a=add('settings','Tài khoản / Tùy chọn');header(a,'Tài khoản');bottom(a,390,844,2);circle(a,61,179,33,RAISED);text(a,51,164,'D',28,ACCENT,600);text(a,110,162,'Không gian cá nhân',18,TEXT,600);text(a,110,192,'Thiết bị đã đăng nhập',12,MUTED)
for i,(title,sub,ico) in enumerate([('Hình ảnh & độ mượt','Tự động · Ưu tiên ổn định','monitor'),('Âm thanh & micro','Mic tắt khi tham gia phiên','mic-off'),('Thao tác cảm ứng','Cử chỉ & bàn phím bổ trợ','touch'),('Chi tiết kết nối','Chỉ số thật sau khi bắt đầu phiên','wifi')]):
 yy=251+i*100;rect(a,24,yy,342,86,SURFACE,17,BORDER);icon(a,ico,42,yy+29,ACCENT,24);text(a,87,yy+22,title,14,TEXT,600);text(a,87,yy+49,sub,10,MUTED);icon(a,'chevron',331,yy+33,MUTED,18)
text(a,24,693,'Bản xem trước giao diện · Chưa phải ứng dụng',11,MUTED);hot(d,0,749,130,76,'home','Máy tính');hot(d,130,749,130,76,'sharing','Chia sẻ')
d,a=add('failure','Kết nối / Không có dữ liệu');header(a,'Chưa nhận được hình',back=True);hot(d,20,61,44,44,'home','Về danh sách máy');circle(a,195,237,61,'#332834');icon(a,'wifi',168,211,RED,54)
text(a,49,336,'Máy tính chưa gửi hình ảnh',21,TEXT,700);text(a,36,378,'Đã tìm thấy máy, nhưng luồng hình chưa sẵn sàng.',12,MUTED);text(a,43,402,'Bạn có thể thử lại mà không đăng nhập lại.',12,MUTED)
rect(a,24,465,342,113,SURFACE,18,BORDER);text(a,42,484,'Trạng thái kết nối',13,TEXT,600);text(a,42,513,'Đã xác thực thiết bị',12,MINT);text(a,42,543,'Chưa nhận khung hình đầu tiên',12,RED)
button(a,24,623,342,'Thử kết nối lại',ico='arrow');hot(d,24,623,342,50,'connecting','Thử lại mô phỏng');button(a,24,685,342,'Về danh sách máy',kind='secondary');hot(d,24,685,342,50,'home','Về danh sách máy');text(a,80,778,'Trạng thái minh họa trong prototype',10,MUTED)
d,a=add('disconnect','Phiên / Xác nhận rời');header(a,'Rời phiên kết nối');icon(a,'leave',166,218,ACCENT,58);text(a,72,339,'Bạn muốn ngắt kết nối?',22,TEXT,700);text(a,66,387,'Máy tính từ xa vẫn tiếp tục hoạt động.',13,MUTED);text(a,63,412,'Voice của phiên này cũng sẽ kết thúc.',13,MUTED)
button(a,24,544,342,'Ngắt kết nối',kind='danger',ico='leave');hot(d,24,544,342,50,'home','Xác nhận ngắt mô phỏng');button(a,24,608,342,'Tiếp tục điều khiển',kind='secondary');hot(d,24,608,342,50,'remote','Tiếp tục phiên')
# Assign Figma page coordinates; all nodes remain editable native layers.
xx=0;yy=160;row_height=0
for d in SCREENS:
 if xx+d['width']>1800:xx=0;yy+=row_height+110;row_height=0
 d['x']=xx;d['y']=yy;xx+=d['width']+60;row_height=max(row_height,d['height'])
DOC=dict(name='Sanser Mobile · Concept 01',version=1,description='UI preview only. All sessions, people and metrics are fictional. No real connection or microphone access.',colors=dict(background=BG,surface=SURFACE,text=TEXT,muted=MUTED,accent=ACCENT,mint=MINT),screens=SCREENS)

def svg(d):
 out=[f'<svg xmlns="http://www.w3.org/2000/svg" width="{d["width"]}" height="{d["height"]}" viewBox="0 0 {d["width"]} {d["height"]}">',f'<title>{escape(d["name"])}</title>']
 for index,n in enumerate(d['nodes']):
  t=n['type'];stroke=f' stroke="{n["stroke"]}" stroke-width="1"' if n.get('stroke') else ''
  if t=='clip-start':out.append(f'<defs><clipPath id="clip-{index}"><rect x="{n["x"]}" y="{n["y"]}" width="{n["w"]}" height="{n["h"]}" rx="{n["r"]}"/></clipPath></defs><g clip-path="url(#clip-{index})">')
  elif t=='clip-end':out.append('</g>')
  elif t=='rect':out.append(f'<rect x="{n["x"]}" y="{n["y"]}" width="{n["w"]}" height="{n["h"]}" rx="{n["r"]}" fill="{n["fill"]}"{stroke}/>')
  elif t=='ellipse':out.append(f'<ellipse cx="{n["x"]+n["w"]/2}" cy="{n["y"]+n["h"]/2}" rx="{n["w"]/2}" ry="{n["h"]/2}" fill="{n["fill"]}"{stroke}/>')
  elif t=='line':out.append(f'<line x1="{n["x"]}" y1="{n["y"]}" x2="{n["x2"]}" y2="{n["y2"]}" stroke="{n["color"]}" stroke-width="{n["width"]}" stroke-linecap="round"/>')
  elif t=='text':out.append(f'<text x="{n["x"]}" y="{n["y"]+n["size"]*.87}" fill="{n["color"]}" font-family="Inter,Arial,sans-serif" font-size="{n["size"]}" font-weight="{n["weight"]}">{escape(n["text"])}</text>')
 return '\n'.join(out+['</svg>'])

def png(d,scale=2):
 im=Image.new('RGB',(int(d['width']*scale),int(d['height']*scale)),BG);dr=ImageDraw.Draw(im);stack=[]
 for n in d['nodes']:
  t=n['type'];xy=[n['x']*scale,n['y']*scale]
  if t=='clip-start':
   stack.append((im,n));im=Image.new('RGB',im.size,BG);dr=ImageDraw.Draw(im);continue
  elif t=='clip-end':
   parent,clip=stack.pop();mask=Image.new('L',im.size,0);md=ImageDraw.Draw(mask);md.rounded_rectangle([clip['x']*scale,clip['y']*scale,(clip['x']+clip['w'])*scale,(clip['y']+clip['h'])*scale],radius=clip['r']*scale,fill=255);parent.paste(im,(0,0),mask);im=parent;dr=ImageDraw.Draw(im);continue
  if t in ['rect','ellipse']:
   box=xy+[(n['x']+n['w'])*scale,(n['y']+n['h'])*scale]
   if t=='rect':dr.rounded_rectangle(box,radius=n['r']*scale,fill=n['fill'],outline=n.get('stroke'),width=max(1,round(scale)))
   else:dr.ellipse(box,fill=n['fill'],outline=n.get('stroke'),width=max(1,round(scale)))
  elif t=='line':dr.line(xy+[n['x2']*scale,n['y2']*scale],fill=n['color'],width=max(1,round(n['width']*scale)))
  else:
   font='/System/Library/Fonts/Supplemental/Arial Bold.ttf' if n['weight']>=600 else '/System/Library/Fonts/Supplemental/Arial.ttf'
   f=ImageFont.truetype(font,round(n['size']*scale));dr.text(tuple(xy),n['text'],font=f,fill=n['color'],anchor='lt')
 return im

(ROOT/'screens').mkdir(exist_ok=True)
for d in SCREENS:(ROOT/'screens'/f'{d["id"]}.svg').write_text(svg(d))
(ROOT/'design.json').write_text(json.dumps(DOC,ensure_ascii=False,indent=2))
# Preview board: four core portrait states, two landscape states.
board=dict(width=1844,height=1610,name='Sanser Mobile · UI/UX concept 01',nodes=[]);a=board['nodes'];rect(a,0,0,1844,1610,'#080C13',0);text(a,52,38,'SANSER / MOBILE',16,MINT,600);text(a,52,79,'Máy tính của bạn. Luôn trong tầm tay.',40,TEXT,700);text(a,53,143,'Android + iOS  /  Concept 01  /  Tất cả dữ liệu kết nối và voice là minh họa',16,MUTED)
for i,id in enumerate(['home','device','sharing','voice']):
 d=next(s for s in SCREENS if s['id']==id);xx=52+i*448;yy=244
 text(a,xx,209,f'0{i+1}   '+d['name'],14,MUTED,600)
 for n in d['nodes']:
  n=copy.deepcopy(n);n['x']+=xx;n['y']+=yy
  if n['type']=='line':n['x2']+=xx;n['y2']+=yy
  a.append(n)
for i,id in enumerate(['remote','viewer']):
 d=next(s for s in SCREENS if s['id']==id);xx=52+i*895;yy=1190;text(a,xx,1149,f'0{i+5}   '+d['name'],14,MUTED,600)
 for n in d['nodes']:
  n=copy.deepcopy(n);n['x']+=xx;n['y']+=yy
  if n['type']=='line':n['x2']+=xx;n['y2']+=yy
  a.append(n)
(ROOT/'overview.svg').write_text(svg(board));png(board,1.5).save(ROOT/'overview.png')
png(next(s for s in SCREENS if s['id']=='android-home'),2).save(ROOT/'android-preview.png')
# Standalone clickable prototype, no libraries, accounts or device permissions.
html='''<!doctype html><html lang="vi"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Sanser Mobile · Concept 01</title><style>
*{box-sizing:border-box}body{margin:0;background:#080c13;color:#f2f4fa;font:14px system-ui,Arial}header{padding:24px 32px;border-bottom:1px solid #28334a;display:flex;gap:20px;align-items:center}header b{font-size:21px}header span{color:#9eaac0}main{display:flex;min-height:calc(100vh - 81px)}aside{padding:24px;width:245px;flex-shrink:0}aside button{display:block;width:100%;text-align:left;margin:7px 0;padding:12px;border:1px solid #303a50;background:#191f2c;color:#ccd6eb;border-radius:10px;cursor:pointer}aside button[aria-current=true]{border-color:#b9c8ff;color:#b9c8ff}section{display:flex;flex:1;align-items:flex-start;justify-content:center;padding:28px;overflow:auto}.stage{position:relative;flex-shrink:0;border-radius:30px;overflow:hidden;box-shadow:0 20px 70px #0008;outline:1px solid #354158}.stage>svg{display:block;width:100%;height:100%}.hit{position:absolute;background:transparent;border:0;cursor:pointer;border-radius:12px}.hit:hover{background:#b9c8ff16;outline:1px solid #b9c8ff66}.hit:focus-visible{outline:3px solid #78dfc0}.note{font-size:12px;color:#9eaac0;line-height:1.7;margin-top:20px}a{color:#b9c8ff}@media(max-width:700px){header{padding:16px;font-size:12px;display:block}header span{display:block;margin-top:8px}main{display:block}aside{width:100%;display:flex;gap:6px;overflow:auto;padding:12px}aside button{width:auto;white-space:nowrap}aside .note{display:none}section{padding:16px}.stage{max-width:100%}}</style><header><b>Sanser Mobile</b><span>Concept 01 · Chỉ xem trước UI/UX · Không truy cập máy tính hoặc mic</span></header><main><aside id="nav"></aside><section><div id="stage" class="stage"></div></section></main><script>const doc=__DOC__;const svgMap=__SVGS__;let current='home';
const labels=['home','android-home','device','connecting','remote','sharing','viewer','voice','settings','failure'];
function show(id){const d=doc.screens.find(s=>s.id===id);if(!d)return;current=id;const stage=document.getElementById('stage');stage.style.width=d.width+'px';stage.style.aspectRatio=d.width+'/'+d.height;stage.innerHTML=svgMap[id];for(const h of d.hotspots){const b=document.createElement('button');b.className='hit';b.style.left=100*h.x/d.width+'%';b.style.top=100*h.y/d.height+'%';b.style.width=100*h.w/d.width+'%';b.style.height=100*h.h/d.height+'%';b.title=h.label;b.setAttribute('aria-label',h.label);b.onclick=()=>show(h.to);stage.append(b)}for(const b of document.querySelectorAll('aside button'))b.setAttribute('aria-current',b.dataset.screen===id);history.replaceState(null,'','#'+id)}
const nav=document.getElementById('nav');for(const id of labels){const d=doc.screens.find(s=>s.id===id);const b=document.createElement('button');b.textContent=d.name;b.dataset.screen=id;b.onclick=()=>show(id);nav.append(b)}const note=document.createElement('p');note.className='note';note.textContent='Nhấn các vùng tương tác trên màn hình để thử luồng. RTT, người tham gia và trạng thái mic đều là dữ liệu mô phỏng.';nav.append(note);show(doc.screens.some(s=>s.id===location.hash.slice(1))?location.hash.slice(1):'home');</script></html>'''
html=html.replace('__DOC__',json.dumps(DOC,ensure_ascii=False)).replace('__SVGS__',json.dumps({d['id']:svg(d) for d in SCREENS},ensure_ascii=False));(ROOT/'preview.html').write_text(html)
# Figma development plugin creates actual Frame/Text/Rectangle/Ellipse layers.
plugin='''const doc=__DOC__;
function paint(hex){return [{type:'SOLID',color:{r:parseInt(hex.slice(1,3),16)/255,g:parseInt(hex.slice(3,5),16)/255,b:parseInt(hex.slice(5,7),16)/255}}]}
async function main(){const page=figma.createPage();page.name=doc.name;await figma.setCurrentPageAsync(page);const fonts=await figma.listAvailableFontsAsync();const family=fonts.some(f=>f.fontName.family==='Inter')?'Inter':'Arial';const regular={family,style:'Regular'};const bold={family,style:fonts.some(f=>f.fontName.family===family&&f.fontName.style==='Semi Bold')?'Semi Bold':'Bold'};await figma.loadFontAsync(regular);await figma.loadFontAsync(bold);const frames=new Map();
for(const s of doc.screens){const f=figma.createFrame();f.name=s.name;f.x=s.x;f.y=s.y;f.resize(s.width,s.height);f.fills=paint(doc.colors.background);f.clipsContent=true;f.cornerRadius=24;page.appendChild(f);frames.set(s.id,f);let parent=f,ox=0,oy=0;for(const n of s.nodes){if(n.type==='clip-start'){const c=figma.createFrame();c.name='Desktop preview / clipped';c.resize(n.w,n.h);c.cornerRadius=n.r;c.clipsContent=true;c.fills=[];f.appendChild(c);c.x=n.x;c.y=n.y;parent=c;ox=n.x;oy=n.y;continue}if(n.type==='clip-end'){parent=f;ox=0;oy=0;continue}let node;if(n.type==='text'){node=figma.createText();node.fontName=n.weight>=600?bold:regular;node.fontSize=n.size;node.characters=n.text;node.fills=paint(n.color);node.textAutoResize='WIDTH_AND_HEIGHT';node.name=n.text}else if(n.type==='line'){node=figma.createLine();const dx=n.x2-n.x,dy=n.y2-n.y;node.resize(Math.max(.01,Math.hypot(dx,dy)),0);node.rotation=-Math.atan2(dy,dx)*180/Math.PI;node.strokes=paint(n.color);node.strokeWeight=n.width;node.strokeCap='ROUND';node.name='Icon / stroke'}else{node=n.type==='ellipse'?figma.createEllipse():figma.createRectangle();node.resize(n.w,n.h);node.fills=paint(n.fill);if(n.type==='rect')node.cornerRadius=Math.min(n.r,n.w/2,n.h/2);if(n.stroke){node.strokes=paint(n.stroke);node.strokeWeight=1}else node.strokes=[];node.name=n.type==='ellipse'?'Icon / dot':'Surface'}parent.appendChild(node);node.x=n.x-ox;node.y=n.y-oy}}
for(const s of doc.screens){const f=frames.get(s.id);for(const h of s.hotspots){const n=figma.createRectangle();n.name='Tap / '+h.label;n.resize(h.w,h.h);n.fills=[{type:'SOLID',color:{r:1,g:1,b:1},opacity:0}];f.appendChild(n);n.x=h.x;n.y=h.y;const dest=frames.get(h.to);if(dest)await n.setReactionsAsync([{trigger:{type:'ON_CLICK'},actions:[{type:'NODE',destinationId:dest.id,navigation:'NAVIGATE',transition:null,preserveScrollPosition:false}]}])}}
page.selection=[frames.get('home')];figma.viewport.scrollAndZoomIntoView([frames.get('home'),frames.get('android-home')]);figma.closePlugin('Created '+doc.screens.length+' editable Sanser Mobile screens. All sessions and measurements are demo data.');}
main().catch(e=>{figma.closePlugin('Design creation failed: '+e.message)});'''
(ROOT/'figma-plugin'/'code.js').write_text(plugin.replace('__DOC__',json.dumps(DOC,ensure_ascii=False)))
(ROOT/'figma-plugin'/'manifest.template.json').write_text(json.dumps(dict(name='Sanser Mobile · Concept 01',api='1.0.0',main='code.js',editorType=['figma'],documentAccess='dynamic-page',networkAccess=dict(allowedDomains=['none'])),indent=2))
print('Generated',len(SCREENS),'screens, editable Figma plugin, PNG overview, SVGs and clickable preview.')
