package main

import (
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/url"
	"os"
	"path/filepath"
	"sync/atomic"
	"time"

	"github.com/go-logr/stdr"
	"github.com/gorilla/websocket"
	"github.com/livekit/protocol/livekit"
	protoLogger "github.com/livekit/protocol/logger"
	lksdk "github.com/livekit/server-sdk-go/v2"
	"github.com/pion/webrtc/v4"
	"google.golang.org/protobuf/proto"
)

type exchange struct {
	URL           string   `json:"url"`
	Tokens        []string `json:"tokens"`
	Tone          string   `json:"tone"`
	SampleSeconds int      `json:"sampleSeconds"`
}

func write(path string, value any) error {
	data, err := json.Marshal(value)
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0600)
}

// Obtain an actual SFU-refreshed JWT (not a fabricated refreshed claim). The
// temporary signaling-only peer opens no capture/output or media device.
func refreshedToken(base, token string) (string, error) {
	u, _ := url.Parse(base + "/rtc")
	q := u.Query()
	q.Set("access_token", token)
	q.Set("protocol", "15")
	q.Set("sdk", "go")
	q.Set("auto_subscribe", "0")
	u.RawQuery = q.Encode()
	conn, _, err := websocket.DefaultDialer.Dial(u.String(), nil)
	if err != nil {
		return "", errors.New("refresh probe join failed")
	}
	defer conn.Close()
	_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	for {
		_, data, err := conn.ReadMessage()
		if err != nil {
			return "", errors.New("refresh probe deadline")
		}
		var response livekit.SignalResponse
		if proto.Unmarshal(data, &response) == nil && response.GetRefreshToken() != "" {
			return response.GetRefreshToken(), nil
		}
	}
}
func run(dir string) error {
	lksdk.SetLogger(protoLogger.LogRLogger(stdr.New(log.New(io.Discard, "", 0))))
	path := filepath.Join(dir, "credentials.json")
	deadline := time.Now().Add(45 * time.Second)
	var input exchange
	for {
		data, err := os.ReadFile(path)
		if err == nil {
			if json.Unmarshal(data, &input) == nil {
				break
			}
		}
		if time.Now().After(deadline) {
			return errors.New("credential deadline")
		}
		time.Sleep(50 * time.Millisecond)
	}
	if input.URL != "ws://127.0.0.1:17883" || len(input.Tokens) != 3 {
		return errors.New("test endpoints/participants rejected")
	}
	if input.SampleSeconds < 10 || input.SampleSeconds > 600 {
		return errors.New("sample duration rejected")
	}
	refreshed, err := refreshedToken(input.URL, input.Tokens[0])
	if err != nil {
		return err
	}
	var packets [3]atomic.Int64
	var tracks [3]atomic.Int64
	var dropped [3]atomic.Bool
	var lastPacket [3][2]atomic.Int64
	var maxGap [3]atomic.Int64
	var rooms []*lksdk.Room
	defer func() {
		for _, room := range rooms {
			room.Disconnect()
		}
	}()
	for index, token := range input.Tokens {
		i := index
		callback := lksdk.NewRoomCallback()
		callback.OnDisconnected = func() { dropped[i].Store(true) }
		callback.OnTrackSubscribed = func(track *webrtc.TrackRemote, _ *lksdk.RemoteTrackPublication, _ *lksdk.RemoteParticipant) {
			if track.Codec().MimeType != webrtc.MimeTypeOpus {
				return
			}
			slot := tracks[i].Add(1) - 1
			if slot >= 2 {
				return
			}
			go func() {
				for {
					packet, _, err := track.ReadRTP()
					if err != nil {
						return
					}
					if len(packet.Payload) > 0 {
						packets[i].Add(1)
						now := time.Now().UnixNano()
						previous := lastPacket[i][slot].Swap(now)
						if previous > 0 {
							gap := now - previous
							for old := maxGap[i].Load(); gap > old; old = maxGap[i].Load() {
								if maxGap[i].CompareAndSwap(old, gap) {
									break
								}
							}
						}
					}
				}
			}()
		}
		room, err := lksdk.ConnectToRoomWithToken(input.URL, token, callback)
		if err != nil {
			return errors.New("peer join failed")
		}
		rooms = append(rooms, room)
	}
	for _, room := range rooms {
		track, err := lksdk.NewLocalFileTrack(input.Tone)
		if err != nil {
			return errors.New("synthetic Opus open failed")
		}
		defer track.Close()
		_, err = room.LocalParticipant.PublishTrack(track, &lksdk.TrackPublicationOptions{Name: "synthetic-tone", Source: livekit.TrackSource_MICROPHONE, DisableDTX: true})
		if err != nil {
			return errors.New("synthetic Opus publish failed")
		}
	}
	sampleStart := time.Now()
	sampleDuration := time.Duration(input.SampleSeconds) * time.Second
	deadline = sampleStart.Add(sampleDuration + 10*time.Second)
	for {
		ready := time.Since(sampleStart) >= sampleDuration
		for i := range 3 {
			if dropped[i].Load() {
				return errors.New("peer disconnected during sample")
			}
			if time.Since(sampleStart) > 5*time.Second {
				for slot := range 2 {
					last := lastPacket[i][slot].Load()
					if last == 0 || time.Since(time.Unix(0, last)) > 3*time.Second {
						return errors.New("remote Opus track stalled during sample")
					}
				}
			}
			ready = ready && packets[i].Load() >= int64(input.SampleSeconds)*60 && tracks[i].Load() == 2
		}
		if ready {
			break
		}
		if time.Now().After(deadline) {
			return errors.New("RTP deadline")
		}
		time.Sleep(50 * time.Millisecond)
	}
	sampleMs := time.Since(sampleStart).Milliseconds()
	if err := write(filepath.Join(dir, "ready.json"), map[string]any{"connected": 3, "sampleDurationMs": sampleMs, "receivedTracks": []int64{tracks[0].Load(), tracks[1].Load(), tracks[2].Load()}, "rtpPackets": []int64{packets[0].Load(), packets[1].Load(), packets[2].Load()}}); err != nil {
		return err
	}
	deadline = time.Now().Add(15 * time.Second)
	for {
		ready := true
		for i := range 3 {
			ready = ready && dropped[i].Load()
		}
		if ready {
			break
		}
		if time.Now().After(deadline) {
			return errors.New("disconnect deadline")
		}
		time.Sleep(25 * time.Millisecond)
	}
	// Deliberately test the documented self-hosted replay limitation. Do not
	// mistake room deletion for token revocation or publish new data there.
	replay, err := lksdk.ConnectToRoomWithToken(input.URL, input.Tokens[0], lksdk.NewRoomCallback())
	replayJoined := err == nil
	if replayJoined {
		replay.Disconnect()
	}
	refreshedPeer, err := lksdk.ConnectToRoomWithToken(input.URL, refreshed, lksdk.NewRoomCallback())
	refreshedJoined := err == nil
	if refreshedJoined {
		refreshedPeer.Disconnect()
	}
	return write(filepath.Join(dir, "result.json"), map[string]any{"connected": 3, "disconnected": 3, "sampleDurationMs": sampleMs, "receivedTracks": []int64{tracks[0].Load(), tracks[1].Load(), tracks[2].Load()}, "rtpPackets": []int64{packets[0].Load(), packets[1].Load(), packets[2].Load()}, "maxRtpGapMs": []float64{float64(maxGap[0].Load()) / 1e6, float64(maxGap[1].Load()) / 1e6, float64(maxGap[2].Load()) / 1e6}, "cachedTokenRejoinSucceeded": replayJoined, "refreshedTokenRejoinSucceeded": refreshedJoined})
}
func main() {
	if len(os.Args) != 2 {
		os.Exit(2)
	}
	if err := run(os.Args[1]); err != nil {
		_ = write(filepath.Join(os.Args[1], "error.json"), map[string]string{"error": err.Error()})
		os.Exit(1)
	}
}
