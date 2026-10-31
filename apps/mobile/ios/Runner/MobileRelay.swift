import Foundation
import CryptoKit
import Security
import Clibsodium
import Darwin

private func uuidBytes(_ value: String) throws -> [UInt8] {
    guard let id = UUID(uuidString: value) else { throw MobileError.invalidConfiguration }
    var bytes = id.uuid
    return withUnsafeBytes(of: &bytes) { Array($0) }
}
enum MobileError: Error { case invalidConfiguration, crypto, socket }
final class MobileRelay {
    private let socket: Int32
    private let source: DispatchSourceRead
    private let queue = DispatchQueue(label: "SanserRelay")
    private let cipherKey: [UInt8], session: [UInt8], own: [UInt8], peer: [UInt8]
    private var prefix = [UInt8](repeating: 0, count: 16)
    private var remotePrefix: [UInt8]?, received: UInt64?, sequence: UInt64 = 0
    private var engineAddress: sockaddr_in?
    private var web: URLSessionWebSocketTask?
    private let http = URLSession(configuration: .ephemeral)
    private var closed = false, announced = false
    private var pendingBytes = 0
    var ready: ((String) -> Void)?
    var failed: ((String) -> Void)?
    let endpoint: String
    private let url: URL, access: String
    init(_ c: [String: Any]) throws {
        guard let raw = c["serverUrl"] as? String, var parts = URLComponents(string: raw), parts.scheme == "https", parts.user == nil, parts.password == nil, parts.query == nil, parts.fragment == nil,
              let sessionID=c["sessionId"] as? String, let device=c["deviceId"] as? String, let other=c["peerDeviceId"] as? String,
              let token=c["sessionToken"] as? String, let bearer=c["accessToken"] as? String, !bearer.contains("\r"), !bearer.contains("\n") else {throw MobileError.invalidConfiguration}
        session=try uuidBytes(sessionID);own=try uuidBytes(device);peer=try uuidBytes(other)
        var material=Array("sanser-relay-frame-key-v1\0".utf8);material+=session;material+=Array(token.utf8)
        cipherKey=Array(SHA256.hash(data: Data(material)));access=bearer
        guard sodium_init() >= 0, SecRandomCopyBytes(kSecRandomDefault,16,&prefix)==errSecSuccess else {throw MobileError.crypto}
        parts.scheme="wss";parts.path=parts.path.trimmingCharacters(in: CharacterSet(charactersIn:"/"))
        parts.path = (parts.path.isEmpty ? "" : "/"+parts.path)+"/api/v2/relay"
        parts.queryItems=[URLQueryItem(name:"sessionId",value:sessionID),URLQueryItem(name:"deviceId",value:device)]
        guard let relayURL=parts.url else {throw MobileError.invalidConfiguration};url=relayURL
        let fd=Darwin.socket(AF_INET,SOCK_DGRAM,IPPROTO_UDP)
        socket=fd
        guard fd>=0 else {throw MobileError.socket}
        var address=sockaddr_in();address.sin_len=UInt8(MemoryLayout<sockaddr_in>.size);address.sin_family=sa_family_t(AF_INET);address.sin_addr.s_addr=inet_addr("127.0.0.1")
        let bound=withUnsafePointer(to:&address){$0.withMemoryRebound(to:sockaddr.self,capacity:1){Darwin.bind(fd,$0,socklen_t(MemoryLayout<sockaddr_in>.size))}}
        guard bound==0 else {Darwin.close(fd);throw MobileError.socket}
        var length=socklen_t(MemoryLayout<sockaddr_in>.size)
        _=withUnsafeMutablePointer(to:&address){$0.withMemoryRebound(to:sockaddr.self,capacity:1){getsockname(fd,$0,&length)}}
        endpoint="127.0.0.1:\(UInt16(bigEndian:address.sin_port))"
        _=fcntl(fd,F_SETFL,O_NONBLOCK)
        source=DispatchSource.makeReadSource(fileDescriptor:socket,queue:queue)
        source.setCancelHandler{Darwin.close(fd)}
        source.setEventHandler{[weak self] in self?.readDatagrams()}
    }
    func start(){queue.async{[weak self] in
        guard let self=self,!self.closed else{return}
        var request=URLRequest(url:self.url);request.setValue("sanser-relay-v1, bearer.\(self.access)",forHTTPHeaderField:"Sec-WebSocket-Protocol")
        self.web=self.http.webSocketTask(with:request);self.web?.resume();self.source.resume();self.receive()
    }}
    private func receive(){web?.receive{[weak self] result in self?.queue.async{[weak self] in
        guard let self=self,!self.closed else{return}
        switch result {
        case .failure: self.fail("Kết nối relay đã ngắt.")
        case .success(let message):
            switch message {
            case .string(let text):
                guard text.utf8.count<=16384 else{return self.fail("Phản hồi relay không hợp lệ.")}
                let object=(try? JSONSerialization.jsonObject(with:Data(text.utf8))) as? [String:Any]
                if object?["type"] as? String == "relay.peerReady",!self.announced {self.announced=true;self.ready?(self.endpoint)}
                if object?["type"] as? String == "relay.peerOffline" {return self.fail("Máy tính đã rời phiên.")}
            case .data(let frame):
                if let data=self.open(Array(frame)),var address=self.engineAddress {
                    _=data.withUnsafeBytes{p in withUnsafePointer(to:&address){$0.withMemoryRebound(to:sockaddr.self,capacity:1){sendto(self.socket,p.baseAddress,data.count,0,$0,socklen_t(MemoryLayout<sockaddr_in>.size))}}}
                }
            @unknown default: break
            }
            self.receive()
        }
    }}}
    private func readDatagrams(){
        while !closed {
            var bytes=[UInt8](repeating:0,count:65536),address=sockaddr_in(),length=socklen_t(MemoryLayout<sockaddr_in>.size)
            let n=withUnsafeMutablePointer(to:&address){$0.withMemoryRebound(to:sockaddr.self,capacity:1){recvfrom(socket,&bytes,bytes.count,0,$0,&length)}}
            if n<0 {return}
            guard n>0,n<=65476,address.sin_addr.s_addr==inet_addr("127.0.0.1"),engineAddress==nil || engineAddress?.sin_port==address.sin_port else{continue}
            engineAddress=address
            guard pendingBytes<2*1024*1024,let frame=seal(Array(bytes.prefix(n))) else{return fail("Hàng đợi relay vượt giới hạn.")}
            pendingBytes+=frame.count
            web?.send(.data(Data(frame))){[weak self] error in self?.queue.async{[weak self] in
                guard let self=self else{return};self.pendingBytes-=frame.count
                if error != nil && !self.closed {self.fail("Không gửi được gói tin relay.")}
            }}
        }
    }
    private func seal(_ packet:[UInt8])->[UInt8]? {
        guard sequence<UInt64.max else{return nil}
        let counter=(0..<8).reversed().map{UInt8(truncatingIfNeeded:sequence >> ($0*8))};sequence+=1
        let header=Array("SNR1".utf8)+own+prefix+counter,nonce=prefix+counter,aad=header+session
        var output=[UInt8](repeating:0,count:packet.count+16),length:UInt64=0
        guard crypto_aead_xchacha20poly1305_ietf_encrypt(&output,&length,packet,UInt64(packet.count),aad,UInt64(aad.count),nil,nonce,cipherKey)==0 else{return nil}
        return header+output
    }
    private func open(_ frame:[UInt8])->[UInt8]? {
        guard (61...65536).contains(frame.count),Array(frame[0..<4])==Array("SNR1".utf8),Array(frame[4..<20])==peer else{return nil}
        let counter=frame[36..<44].reduce(UInt64(0)){($0<<8)|UInt64($1)},np=Array(frame[20..<36])
        guard received==nil || counter>received!,remotePrefix==nil || remotePrefix==np else{return nil}
        let encrypted=Array(frame[44...]),nonce=Array(frame[20..<44]),aad=Array(frame[..<44])+session
        var packet=[UInt8](repeating:0,count:encrypted.count-16),length:UInt64=0
        guard crypto_aead_xchacha20poly1305_ietf_decrypt(&packet,&length,nil,encrypted,UInt64(encrypted.count),aad,UInt64(aad.count),nonce,cipherKey)==0 else{return nil}
        received=counter;remotePrefix=np;return packet
    }
    private func fail(_ message:String){guard !closed else{return};failed?(message);shutdown()}
    private func shutdown(){guard !closed else{return};closed=true;if web==nil{source.resume()};source.cancel();web?.cancel(with:.normalClosure,reason:nil);http.invalidateAndCancel()}
    func close(){queue.async{[weak self] in self?.shutdown()}}
    deinit{if !closed{if web==nil{source.resume()};source.cancel()};web?.cancel(with:.normalClosure,reason:nil);http.invalidateAndCancel()}
}
