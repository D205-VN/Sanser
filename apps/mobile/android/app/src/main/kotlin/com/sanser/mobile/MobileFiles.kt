package com.sanser.mobile

import android.app.Activity
import android.content.Intent
import android.provider.OpenableColumns
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors
import org.json.JSONObject

/** Shared Rust worker; JNI returns UTF-8 bytes, not modified UTF-8 strings. */
object MobileFileBridge {
    init { System.loadLibrary("sanser_mobile") }
    private external fun create(root:ByteArray):Long
    private external fun request(handle:Long,json:ByteArray):ByteArray?
    external fun close(handle:Long)
    fun open(root:File):Long=create(root.absolutePath.toByteArray(Charsets.UTF_8))
    fun call(handle:Long,action:Map<String,Any?>):Map<String,Any?> {
        val raw=request(handle,JSONObject(action).toString().toByteArray(Charsets.UTF_8))?:error("File session ended")
        val reply=JSONObject(String(raw,Charsets.UTF_8))
        check(reply.optBoolean("ok")){reply.optString("error","File operation failed")}
        return objectMap(reply.getJSONObject("result"))
    }
    private fun objectMap(value:JSONObject):Map<String,Any?> = value.keys().asSequence().associateWith { key ->
        when(val item=value.get(key)){JSONObject.NULL->null;is JSONObject->objectMap(item);else->item}
    }
}

/** Document-provider access only after a picker. No broad storage permission. */
class MobileFileDocuments(private val activity:Activity) {
    val base=File(activity.filesDir,"SanserFiles").canonicalFile
    private val io=Executors.newSingleThreadExecutor()
    private var pending:Pending?=null
    private var nextCode=49000
    private data class Pending(val code:Int,val mode:String,val result:MethodChannel.Result,val current:()->Boolean,val root:File?=null,val source:File?=null)
    fun workspace():File=File(base,UUID.randomUUID().toString())
    fun pick(root:File,current:()->Boolean,result:MethodChannel.Result){
        if(pending!=null || !current()){result.error("files_busy","Hộp thoại file chưa sẵn sàng.",null);return}
        val code=nextCode++;if(nextCode>65000)nextCode=49000
        pending=Pending(code,"pick",result,current,root)
        try{activity.startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {type="*/*";addCategory(Intent.CATEGORY_OPENABLE)},code)}catch(_:Exception){pending=null;result.error("files_pick","Không mở được trình chọn file.",null)}
    }
    private fun owned(key:String):File? {
        val parts=key.split('/');if(parts.size!=4 || parts[1]!="received" || parts[2]!="Sanser" || parts[3].length<=37 || parts.any{it=="." || it==".." || it.contains('\\')})return null
        try{UUID.fromString(parts[0]);UUID.fromString(parts[3].take(36))}catch(_:Exception){return null}
        val file=File(base,key)
        return try{if(file.isFile && file.canonicalFile==file.absoluteFile && file.canonicalPath.startsWith(base.canonicalPath+File.separator))file else null}catch(_:Exception){null}
    }
    fun inbox(result:MethodChannel.Result){io.execute{
        val items=try{base.walkTopDown().maxDepth(4).take(10000).mapNotNull{file->
            val key=file.relativeTo(base).invariantSeparatorsPath;val saved=owned(key)?:return@mapNotNull null
            mapOf("key" to key,"name" to saved.name.drop(37),"size" to saved.length())
        }.take(256).toList()}catch(_:Exception){emptyList()}
        activity.runOnUiThread{result.success(items)}
    }}
    fun export(key:String,result:MethodChannel.Result){
        val source=owned(key)
        if(pending!=null || source==null){result.error("files_export","Không mở được file đã nhận.",null);return}
        val code=nextCode++;if(nextCode>65000)nextCode=49000
        pending=Pending(code,"export",result,{true},source=source)
        try{activity.startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply{type="application/octet-stream";addCategory(Intent.CATEGORY_OPENABLE);putExtra(Intent.EXTRA_TITLE,source.name.drop(37))},code)}catch(_:Exception){pending=null;result.error("files_export","Không mở được hộp thoại lưu file.",null)}
    }
    fun cancelPick(){val operation=pending;if(operation?.mode=="pick"){pending=null;activity.finishActivity(operation.code);operation.result.error("files_stale","Phiên điều khiển đã kết thúc.",null)}}
    fun result(requestCode:Int,resultCode:Int,data:Intent?):Boolean {
        if(requestCode !in 49000..65000)return false
        val operation=pending?:return true
        // An old picker may still finish after a cancelled session. Do not let
        // its result satisfy a later picker of the other kind.
        if(requestCode!=operation.code)return true
        val uri=data?.data
        if(resultCode!=Activity.RESULT_OK || uri==null){pending=null;operation.result.success(null);return true}
        io.execute{
            var copied:File?=null
            try{
                val response:Any?=if(operation.mode=="export"){
                    val source=operation.source?:error("No source")
                    // SAF supplies the user-selected destination; no raw path is accepted.
                    activity.contentResolver.openOutputStream(uri,"wt").use{output->check(output!=null);source.inputStream().use{input->input.copyTo(output,65536)}}
                    true
                }else{
                    val root=operation.root?:error("No workspace");root.mkdirs()
                    var name="file";var advertisedSize:Long?=null
                    activity.contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME,OpenableColumns.SIZE),null,null,null)?.use{cursor->if(cursor.moveToFirst()){name=cursor.getString(0)?:"file";if(!cursor.isNull(1))advertisedSize=cursor.getLong(1)}}
                    check((advertisedSize?:0)<=25L*1024*1024)
                    val id=UUID.randomUUID().toString();val target=File(root,"source-$id");check(target.createNewFile());copied=target
                    var count=0L
                    val deadline=android.os.SystemClock.elapsedRealtime()+120000L
                    activity.contentResolver.openInputStream(uri).use { input ->
                        check(input != null)
                        target.outputStream().use { output ->
                            val bytes = ByteArray(65536)
                            while (true) {
                                check(operation.current() && android.os.SystemClock.elapsedRealtime() < deadline)
                                val n = input.read(bytes)
                                if (n < 0) break
                                if (n == 0) { Thread.sleep(5); continue }
                                count += n
                                check(count <= 25L * 1024 * 1024)
                                output.write(bytes, 0, n)
                            }
                        }
                    }
                    mapOf("sourceId" to id,"name" to name,"size" to count)
                }
                activity.runOnUiThread{
                    if(pending!==operation || !operation.current()){copied?.delete();return@runOnUiThread}
                    pending=null;operation.result.success(response)
                }
            }catch(_:Exception){copied?.delete();activity.runOnUiThread{if(pending===operation){pending=null;operation.result.error("files_document","Không đọc/lưu được file. Giới hạn 25 MB.",null)}}}
        }
        return true
    }
    fun close(){pending?.result?.error("files_closed","Đã đóng thao tác file.",null);pending=null;io.shutdown()}
}
