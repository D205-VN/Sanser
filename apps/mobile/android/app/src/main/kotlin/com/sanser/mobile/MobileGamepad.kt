package com.sanser.mobile

import android.content.Context
import android.hardware.input.InputManager
import android.os.Handler
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import kotlin.math.max

/** App-local, one physical device, keyboard-mapping snapshots. Button edges
 * flush immediately; axis-only motion occupies one latest-state 16 ms slot. */
internal class MobileGamepad(context: Context, private val main: Handler) : InputManager.InputDeviceListener {
    private val manager = context.getSystemService(InputManager::class.java)
    private var callback: ((Map<String,Any>) -> Unit)? = null
    private var device: Int? = null
    private var keys=0; private var hat=0
    private var lx=0f;private var ly=0f;private var rx=0f;private var ry=0f;private var lt=0f;private var rt=0f
    private var digitalLT=false;private var digitalRT=false
    private var sampled=false;private var epoch=0L;private var pending=false
    private fun eligible(id: Int): Boolean {
        val source=InputDevice.getDevice(id)?.sources ?: return false
        return source and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD ||
            source and InputDevice.SOURCE_JOYSTICK == InputDevice.SOURCE_JOYSTICK
    }
    fun start(emit: (Map<String,Any>) -> Unit) {
        stop();callback=emit;manager.registerInputDeviceListener(this,main);choose()
    }
    fun stop() {
        epoch++;pending=false
        callback=null;device=null
        manager.unregisterInputDeviceListener(this)
        neutral()
    }
    private fun neutral(){keys=0;hat=0;lx=0f;ly=0f;rx=0f;ry=0f;lt=0f;rt=0f;digitalLT=false;digitalRT=false;sampled=false}
    private fun choose(excluding:Int?=null) {
        if(callback==null)return
        if(device==null) device=InputDevice.getDeviceIds().firstOrNull{it!=excluding && eligible(it)}
        emit()
    }
    fun pause(){if(callback!=null){neutral();emit(true);stop()}}
    private fun emit(paused:Boolean=false) {
        callback?.invoke(mapOf("connected" to (device!=null),"sample" to sampled,"buttons" to (keys or hat),
            "paused" to paused,
            "lx" to lx.toDouble(),"ly" to ly.toDouble(),"rx" to rx.toDouble(),"ry" to ry.toDouble(),
            "lt" to max(lt,if(digitalLT)1f else 0f).toDouble(),"rt" to max(rt,if(digitalRT)1f else 0f).toDouble()))
    }
    private fun publish(immediate: Boolean) {
        if(immediate){epoch++;pending=false;emit();return}
        if(pending)return
        pending=true;val token=epoch
        main.postDelayed({if(callback!=null && token==epoch){pending=false;emit()}},16)
    }
    fun key(event: KeyEvent): Boolean {
        if(callback==null || event.deviceId!=device || !eligible(event.deviceId))return false
        val bit=when(event.keyCode){
            KeyEvent.KEYCODE_BUTTON_A->0;KeyEvent.KEYCODE_BUTTON_B->1;KeyEvent.KEYCODE_BUTTON_X->2;KeyEvent.KEYCODE_BUTTON_Y->3
            KeyEvent.KEYCODE_BUTTON_L1->4;KeyEvent.KEYCODE_BUTTON_R1->5;KeyEvent.KEYCODE_BUTTON_START->6;KeyEvent.KEYCODE_BUTTON_SELECT->7
            KeyEvent.KEYCODE_BUTTON_THUMBL->8;KeyEvent.KEYCODE_BUTTON_THUMBR->9
            KeyEvent.KEYCODE_DPAD_UP->10;KeyEvent.KEYCODE_DPAD_DOWN->11;KeyEvent.KEYCODE_DPAD_LEFT->12;KeyEvent.KEYCODE_DPAD_RIGHT->13
            KeyEvent.KEYCODE_BUTTON_L2->14;KeyEvent.KEYCODE_BUTTON_R2->15;else->return false}
        if(event.action!=KeyEvent.ACTION_DOWN && event.action!=KeyEvent.ACTION_UP)return false
        if(event.repeatCount>0)return true
        val down=event.action==KeyEvent.ACTION_DOWN
        if(bit==14)digitalLT=down else if(bit==15)digitalRT=down
        else keys=if(down) keys or (1 shl bit) else keys and (1 shl bit).inv()
        sampled=true;publish(true);return true
    }
    fun motion(event: MotionEvent): Boolean {
        if(callback==null || event.deviceId!=device || event.actionMasked!=MotionEvent.ACTION_MOVE ||
            !event.isFromSource(InputDevice.SOURCE_JOYSTICK))return false
        fun axis(code: Int): Float {val v=event.getAxisValue(code);return if(v.isFinite())v.coerceIn(-1f,1f)else 0f}
        val oldHat=hat;val oldLT=lt;val oldRT=rt
        lx=axis(MotionEvent.AXIS_X);ly=-axis(MotionEvent.AXIS_Y)
        fun stick(primary:Int,fallback:Int):Float = if(InputDevice.getDevice(event.deviceId)?.getMotionRange(primary,event.source)!=null)axis(primary)else axis(fallback)
        rx=stick(MotionEvent.AXIS_Z,MotionEvent.AXIS_RX);ry=-stick(MotionEvent.AXIS_RZ,MotionEvent.AXIS_RY)
        // Only advertised trigger ranges: a missing axis is not a trigger.
        fun trigger(primary:Int,fallback:Int):Float {
            val input=InputDevice.getDevice(event.deviceId)
            val range=input?.getMotionRange(primary,event.source) ?: input?.getMotionRange(fallback,event.source) ?: return 0f
            if(range.range<=0)return 0f
            return ((event.getAxisValue(range.axis)-range.min)/range.range).coerceIn(0f,1f).takeIf{it.isFinite()} ?: 0f
        }
        lt=trigger(MotionEvent.AXIS_LTRIGGER,MotionEvent.AXIS_BRAKE);rt=trigger(MotionEvent.AXIS_RTRIGGER,MotionEvent.AXIS_GAS)
        val hx=axis(MotionEvent.AXIS_HAT_X);val hy=axis(MotionEvent.AXIS_HAT_Y)
        hat=(if(hy<-.5f)1 shl 10 else 0) or (if(hy>.5f)1 shl 11 else 0) or
            (if(hx<-.5f)1 shl 12 else 0) or (if(hx>.5f)1 shl 13 else 0)
        sampled=true
        publish(hat!=oldHat || (lt>=.55f)!=(oldLT>=.55f) || (rt>=.55f)!=(oldRT>=.55f) ||
            (lt<=.35f)!=(oldLT<=.35f) || (rt<=.35f)!=(oldRT<=.35f));return true
    }
    override fun onInputDeviceAdded(id:Int){if(device==null)choose()}
    override fun onInputDeviceChanged(id:Int){
        if(device!=id)return
        device=null;epoch++;pending=false;neutral();emit();choose()
    }
    override fun onInputDeviceRemoved(id:Int){
        if(device!=id)return
        device=null;epoch++;pending=false;neutral();emit();choose(excluding=id)
    }
}
