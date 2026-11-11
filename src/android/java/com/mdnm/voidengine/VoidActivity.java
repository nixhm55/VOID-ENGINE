package com.mdnm.voidengine;

import android.Manifest;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.app.DownloadManager;
import android.app.NativeActivity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.pm.PackageManager;
import android.content.Intent;
import android.content.IntentFilter;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.webkit.CookieManager;
import android.webkit.DownloadListener;
import android.webkit.JavascriptInterface;
import android.webkit.PermissionRequest;
import android.webkit.URLUtil;
import android.webkit.WebChromeClient;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Toast;
import java.io.FileOutputStream;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.Locale;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicInteger;

public final class VoidActivity extends NativeActivity {
    private static final int MEDIA_PERMISSION_REQUEST = 0x564f;
    private static final int STORAGE_PERMISSION_REQUEST = 0x564e;
    private static final int NOTIFICATION_PERMISSION_REQUEST = 0x564d;
    private static final String NOTIFICATION_CHANNEL_ID = "void_web_notifications";
    private static final String MEDIA_CHANNEL_ID = "void_media_playback";
    private static final int MEDIA_NOTIFICATION_ID = 0x564c;
    private static final String MEDIA_ACTION_PREFIX = "com.mdnm.voidengine.MEDIA_";
    private static final String NOTIFICATION_SHIM = "(function(){"
            + "if(window.__voidNotificationShim)return;window.__voidNotificationShim=true;"
            + "var bridgeToken='__VOID_TOKEN__',next=0,pending={};"
            + "window.__voidResolveNotificationPermission=function(id,granted){"
            + "var resolve=pending[id];if(resolve){delete pending[id];resolve(granted?'granted':'denied');}};"
            + "class VoidNotification extends EventTarget{"
            + "constructor(title,options){super();options=options||{};this.title=String(title||'');"
            + "this.body=String(options.body||'');this.tag=String(options.tag||'');"
            + "if(window.VoidNativeNotifications.permission(bridgeToken)==='granted')"
            + "window.VoidNativeNotifications.show(this.title,this.body,bridgeToken);"
            + "setTimeout(()=>this.dispatchEvent(new Event('show')),0);}"
            + "static get permission(){return window.VoidNativeNotifications.permission(bridgeToken);}"
            + "static requestPermission(callback){var id=++next;var result=new Promise(function(resolve){"
            + "pending[id]=resolve;window.VoidNativeNotifications.requestPermission(id,bridgeToken);});"
            + "if(typeof callback==='function')result.then(callback);return result;}"
            + "close(){}}"
            + "window.Notification=VoidNotification;})();";

    private static final String MEDIA_SHIM = "(function(){"
            + "if(window.__voidMediaShim)return;window.__voidMediaShim=true;"
            + "var session=navigator.mediaSession,handlers={},last='',lastPost=0;"
            + "if(!session||!window.VoidNativeMedia)return;"
            + "window.__voidMediaAction=function(action,position){try{if(handlers[action]){"
            + "var detail=action==='seekto'?{seekTime:(position||0)/1000}:action==='seekforward'||action==='seekbackward'?{seekOffset:10}:undefined;"
            + "handlers[action](detail);}"
            + "else if(action==='play'||action==='pause'){var m=document.querySelector('audio,video');"
            + "if(m){if(action==='play')m.play();else m.pause();}}}catch(e){}};"
            + "var original=session.setActionHandler;try{session.setActionHandler=function(action,handler){"
            + "handlers[action]=handler;original.call(session,action,handler);publish();};}catch(e){}"
            + "var descriptor=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(session),'metadata');"
            + "if(descriptor&&descriptor.configurable){try{Object.defineProperty(session,'metadata',{configurable:true,"
            + "get:function(){return descriptor.get.call(session);},set:function(value){descriptor.set.call(session,value);publish();}});}catch(e){}}"
            + "function publish(){var now=Date.now();if(now-lastPost<500)return;lastPost=now;"
            + "var md=session.metadata||{},artwork='',items=md.artwork||[];"
            + "if(items.length){var best=items[items.length-1];artwork=best.src||'';}"
            + "var media=document.querySelector('audio,video'),position=media?media.currentTime||0:0,"
            + "duration=media&&isFinite(media.duration)?media.duration:0,rate=media?media.playbackRate||1:1;"
            + "var data=JSON.stringify({title:md.title||document.title||'',artist:md.artist||'',album:md.album||'',"
            + "artwork:artwork,state:session.playbackState||'none',position:position,duration:duration,rate:rate,"
            + "actions:Object.keys(handlers)});if(data!==last){last=data;VoidNativeMedia.update(data,'__VOID_MEDIA_TOKEN__');}}"
            + "document.addEventListener('play',publish,true);document.addEventListener('pause',publish,true);"
            + "document.addEventListener('loadedmetadata',publish,true);setInterval(publish,1500);publish();})();";

    private PermissionRequest pendingMediaRequest;
    private String[] pendingMediaResources;
    private String[] pendingMediaPermissions;
    private DownloadInfo pendingDownload;
    private WebView notificationWebView;
    private WebView mediaWebView;
    private String notificationOrigin;
    private String notificationBridgeToken;
    private String mediaBridgeToken;
    private volatile String currentNotificationOrigin;
    private MediaSession mediaSession;
    private PendingIntent mediaSessionActivity;
    private boolean mediaActionReceiverRegistered;
    private final BroadcastReceiver mediaActionReceiver = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) {
            String action = intent.getAction();
            if (action == null || !action.startsWith(MEDIA_ACTION_PREFIX)) return;
            dispatchMediaAction(action.substring(MEDIA_ACTION_PREFIX.length()).toLowerCase(Locale.ROOT));
        }
    };
    private ExecutorService artworkExecutor = Executors.newSingleThreadExecutor();
    private final AtomicInteger artworkRequest = new AtomicInteger();
    private String mediaTitle = "";
    private String mediaArtist = "";
    private String mediaAlbum = "";
    private String mediaArtworkUri = "";
    private Bitmap mediaArtwork;
    private String mediaArtworkRequestedUri = "";
    private long mediaPosition;
    private float mediaRate = 1.0f;
    private int mediaState = PlaybackState.STATE_NONE;
    private long mediaActions;

    @Override
    public void onCreate(android.os.Bundle savedInstanceState) {
        copyPackagedConfig();
        super.onCreate(savedInstanceState);
        if (getActionBar() != null) {
            getActionBar().hide();
        }
    }

    private void copyPackagedConfig() {
        try (InputStream input = getAssets().open("void.json");
             FileOutputStream output = openFileOutput("void.json", MODE_PRIVATE)) {
            byte[] buffer = new byte[4096];
            int count;
            while ((count = input.read(buffer)) != -1) {
                output.write(buffer, 0, count);
            }
        } catch (Exception error) {
            android.util.Log.w("VOID", "Could not copy packaged void.json", error);
        }
    }
    private boolean notificationPermissionRequestInFlight;
    private final ArrayList<Integer> pendingNotificationRequests = new ArrayList<>();
    private final AtomicInteger nextNotificationId = new AtomicInteger(1);

    public void configureVoidWebView(final WebView webView, String allowedUrl) {
        notificationWebView = webView;
        mediaWebView = webView;
        notificationOrigin = secureOrigin(allowedUrl);
        notificationBridgeToken = UUID.randomUUID().toString();
        mediaBridgeToken = UUID.randomUUID().toString();
        webView.addJavascriptInterface(
                new VoidNotificationBridge(notificationBridgeToken), "VoidNativeNotifications");
        webView.addJavascriptInterface(new VoidMediaBridge(mediaBridgeToken), "VoidNativeMedia");
        webView.setWebViewClient(new WebViewClient() {
            @Override
            public void onPageStarted(WebView view, String url, Bitmap favicon) {
                super.onPageStarted(view, url, favicon);
                injectNotificationShim(view, url);
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                injectNotificationShim(view, url);
            }
        });

        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public void onPermissionRequest(final PermissionRequest request) {
                runOnUiThread(new Runnable() {
                    @Override
                    public void run() {
                        requestMediaPermission(request);
                    }
                });
            }

            @Override
            public void onPermissionRequestCanceled(PermissionRequest request) {
                if (request == pendingMediaRequest) {
                    clearPendingMediaRequest();
                }
            }
        });

        webView.setDownloadListener(new DownloadListener() {
            @Override
            public void onDownloadStart(String url, String userAgent,
                    String contentDisposition, String mimeType, long contentLength) {
                requestDownload(url, userAgent, contentDisposition, mimeType, webView.getUrl());
            }
        });

        String sessionName = getApplicationInfo().loadLabel(getPackageManager()).toString();
        mediaSession = new MediaSession(this, sessionName);
        mediaSession.setFlags(MediaSession.FLAG_HANDLES_MEDIA_BUTTONS
                | MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS);
        mediaSessionActivity = PendingIntent.getActivity(
                this, 0, new Intent(this, VoidActivity.class),
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        mediaSession.setSessionActivity(mediaSessionActivity);
        IntentFilter mediaActions = new IntentFilter();
        mediaActions.addAction(MEDIA_ACTION_PREFIX + "PLAY");
        mediaActions.addAction(MEDIA_ACTION_PREFIX + "PAUSE");
        mediaActions.addAction(MEDIA_ACTION_PREFIX + "PREVIOUSTRACK");
        mediaActions.addAction(MEDIA_ACTION_PREFIX + "NEXTTRACK");
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(mediaActionReceiver, mediaActions, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(mediaActionReceiver, mediaActions);
        }
        mediaActionReceiverRegistered = true;
        mediaSession.setCallback(new MediaSession.Callback() {
            @Override public void onPlay() { dispatchMediaAction("play"); }
            @Override public void onPause() { dispatchMediaAction("pause"); }
            @Override public void onStop() { dispatchMediaAction("stop"); }
            @Override public void onSkipToNext() { dispatchMediaAction("nexttrack"); }
            @Override public void onSkipToPrevious() { dispatchMediaAction("previoustrack"); }
            @Override public void onSeekTo(long position) {
                dispatchMediaAction("seekto", position);
            }
            @Override public void onFastForward() { dispatchMediaAction("seekforward"); }
            @Override public void onRewind() { dispatchMediaAction("seekbackward"); }
        });
    }

    private void injectNotificationShim(WebView webView, String url) {
        currentNotificationOrigin = secureOrigin(url);
        if (notificationOrigin != null && notificationOrigin.equals(currentNotificationOrigin)) {
            webView.evaluateJavascript(
                    NOTIFICATION_SHIM.replace("__VOID_TOKEN__", notificationBridgeToken), null);
            webView.evaluateJavascript(
                    MEDIA_SHIM.replace("__VOID_MEDIA_TOKEN__", mediaBridgeToken), null);
        }
    }

    private void dispatchMediaAction(String action) {
        dispatchMediaAction(action, -1L);
    }

    private void dispatchMediaAction(String action, long position) {
        if (mediaWebView == null || !isAllowedNotificationOrigin()) {
            return;
        }
        String script = position >= 0
                ? "window.__voidMediaAction&&window.__voidMediaAction('" + action
                        + "'," + position + ")"
                : "window.__voidMediaAction&&window.__voidMediaAction('" + action + "')";
        mediaWebView.evaluateJavascript(script, null);
    }

    private final class VoidMediaBridge {
        private final String bridgeToken;

        VoidMediaBridge(String bridgeToken) {
            this.bridgeToken = bridgeToken;
        }

        @JavascriptInterface
        public void update(final String data, String token) {
            if (!bridgeToken.equals(token) || !isAllowedNotificationOrigin()) {
                return;
            }
            runOnUiThread(new Runnable() {
                @Override public void run() { updateMediaSession(data); }
            });
        }
    }

    private void updateMediaSession(String payload) {
        if (mediaSession == null || !isAllowedNotificationOrigin()) {
            return;
        }
        try {
            JSONObject data = new JSONObject(payload);
            mediaTitle = data.optString("title", "");
            mediaArtist = data.optString("artist", "");
            mediaAlbum = data.optString("album", "");
            String nextArtworkUri = data.optString("artwork", "");
            if (!nextArtworkUri.equals(mediaArtworkUri)) {
                mediaArtwork = null;
                mediaArtworkUriLoaded = "";
                mediaArtworkRequestedUri = "";
                artworkRequest.incrementAndGet();
            }
            mediaArtworkUri = nextArtworkUri;
            mediaPosition = (long) (data.optDouble("position", 0.0) * 1000.0);
            mediaRate = (float) data.optDouble("rate", 1.0);
            if (mediaRate <= 0.0f) mediaRate = 1.0f;

            JSONArray actions = data.optJSONArray("actions");
            mediaActions = 0L;
            if (actions != null) {
                for (int index = 0; index < actions.length(); index++) {
                    String action = actions.optString(index);
                    if ("play".equals(action)) mediaActions |= PlaybackState.ACTION_PLAY;
                    else if ("pause".equals(action)) mediaActions |= PlaybackState.ACTION_PAUSE;
                    else if ("stop".equals(action)) mediaActions |= PlaybackState.ACTION_STOP;
                    else if ("nexttrack".equals(action)) mediaActions |= PlaybackState.ACTION_SKIP_TO_NEXT;
                    else if ("previoustrack".equals(action)) mediaActions |= PlaybackState.ACTION_SKIP_TO_PREVIOUS;
                    else if ("seekto".equals(action)) mediaActions |= PlaybackState.ACTION_SEEK_TO;
                    else if ("seekforward".equals(action)) mediaActions |= PlaybackState.ACTION_FAST_FORWARD;
                    else if ("seekbackward".equals(action)) mediaActions |= PlaybackState.ACTION_REWIND;
                }
            }
            if (mediaActions == 0L) {
                mediaActions = PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE;
            }

            String playback = data.optString("state", "none");
            mediaState = "playing".equals(playback) ? PlaybackState.STATE_PLAYING
                    : "paused".equals(playback) ? PlaybackState.STATE_PAUSED
                    : "buffering".equals(playback) ? PlaybackState.STATE_BUFFERING
                    : "none".equals(playback) ? PlaybackState.STATE_NONE
                    : PlaybackState.STATE_PAUSED;

            refreshMediaMetadata();
            mediaSession.setPlaybackState(new PlaybackState.Builder()
                    .setActions(mediaActions)
                    .setState(mediaState, mediaPosition, mediaRate)
                    .build());
            boolean active = !mediaTitle.isEmpty() && mediaState != PlaybackState.STATE_NONE;
            mediaSession.setActive(active);
            if (active) {
                showMediaNotification();
            } else {
                NotificationManager manager = (NotificationManager)
                        getSystemService(NOTIFICATION_SERVICE);
                if (manager != null) manager.cancel(MEDIA_NOTIFICATION_ID);
            }
            loadMediaArtwork(mediaArtworkUri);
        } catch (Exception ignored) {
            // Ignore malformed page metadata; media playback itself is unaffected.
        }
    }

    private void refreshMediaMetadata() {
        MediaMetadata.Builder metadata = new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, mediaTitle)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, mediaArtist)
                .putString(MediaMetadata.METADATA_KEY_ALBUM, mediaAlbum);
        if (!mediaArtworkUri.isEmpty()) {
            metadata.putString(MediaMetadata.METADATA_KEY_ART_URI, mediaArtworkUri)
                    .putString(MediaMetadata.METADATA_KEY_ALBUM_ART_URI, mediaArtworkUri);
        }
        if (mediaArtwork != null) {
            metadata.putBitmap(MediaMetadata.METADATA_KEY_ART, mediaArtwork)
                    .putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, mediaArtwork);
        }
        mediaSession.setMetadata(metadata.build());
    }

    private void showMediaNotification() {
        NotificationManager manager = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        // Media-session notifications are exempt from Android 13's regular
        // notification runtime permission, so playback controls stay available
        // independently of website notification permission.
        if (manager == null) return;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(new NotificationChannel(
                    MEDIA_CHANNEL_ID, "Media playback", NotificationManager.IMPORTANCE_LOW));
        }
        Notification.Builder builder = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O
                ? new Notification.Builder(this, MEDIA_CHANNEL_ID)
                : new Notification.Builder(this);
        builder.setSmallIcon(android.R.drawable.ic_media_play)
                .setContentTitle(mediaTitle.isEmpty() ? "Media playback" : mediaTitle)
                .setContentText(mediaArtist)
                .setContentIntent(mediaSessionActivity)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setOngoing(mediaState == PlaybackState.STATE_PLAYING);
        ArrayList<Integer> compactActions = new ArrayList<>();
        if ((mediaActions & PlaybackState.ACTION_SKIP_TO_PREVIOUS) != 0) {
            builder.addAction(mediaNotificationAction(
                    android.R.drawable.ic_media_previous, "Previous", "PREVIOUSTRACK"));
            compactActions.add(0);
        }
        int playPauseIndex = compactActions.size();
        boolean playing = mediaState == PlaybackState.STATE_PLAYING;
        builder.addAction(mediaNotificationAction(
                playing ? android.R.drawable.ic_media_pause : android.R.drawable.ic_media_play,
                playing ? "Pause" : "Play", playing ? "PAUSE" : "PLAY"));
        compactActions.add(playPauseIndex);
        if ((mediaActions & PlaybackState.ACTION_SKIP_TO_NEXT) != 0) {
            builder.addAction(mediaNotificationAction(
                    android.R.drawable.ic_media_next, "Next", "NEXTTRACK"));
            compactActions.add(compactActions.size());
        }
        int[] compact = new int[compactActions.size()];
        for (int index = 0; index < compact.length; index++) compact[index] = compactActions.get(index);
        builder.setStyle(new Notification.MediaStyle()
                .setMediaSession(mediaSession.getSessionToken())
                .setShowActionsInCompactView(compact));
        try {
            manager.notify(MEDIA_NOTIFICATION_ID, builder.build());
        } catch (SecurityException ignored) {
            // Android 13+ requires the user to allow notification display.
        }
    }

    private Notification.Action mediaNotificationAction(int icon, String title, String command) {
        Intent intent = new Intent(MEDIA_ACTION_PREFIX + command).setPackage(getPackageName());
        PendingIntent pendingIntent = PendingIntent.getBroadcast(
                this, command.hashCode(), intent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Action.Builder(icon, title, pendingIntent).build();
    }

    private void loadMediaArtwork(final String artworkUri) {
        if (artworkUri.isEmpty() || (artworkUri.equals(mediaArtworkUriLoaded)
                && mediaArtwork != null)
                || artworkUri.equals(mediaArtworkRequestedUri)) return;
        Uri uri = Uri.parse(artworkUri);
        if (!"https".equalsIgnoreCase(uri.getScheme())
                && !"http".equalsIgnoreCase(uri.getScheme())) return;
        mediaArtworkRequestedUri = artworkUri;
        final int requestId = artworkRequest.incrementAndGet();
        artworkExecutor.execute(new Runnable() {
            @Override public void run() {
                Bitmap bitmap = downloadArtwork(artworkUri);
                if (bitmap == null) return;
                final Bitmap result = bitmap;
                runOnUiThread(new Runnable() {
                    @Override public void run() {
                        if (requestId != artworkRequest.get() || mediaSession == null) return;
                        mediaArtwork = result;
                        mediaArtworkUriLoaded = artworkUri;
                        refreshMediaMetadata();
                        showMediaNotification();
                    }
                });
            }
        });
    }

    private String mediaArtworkUriLoaded = "";

    private static Bitmap downloadArtwork(String rawUrl) {
        HttpURLConnection connection = null;
        try {
            connection = (HttpURLConnection) new URL(rawUrl).openConnection();
            connection.setConnectTimeout(5000);
            connection.setReadTimeout(7000);
            connection.setInstanceFollowRedirects(true);
            connection.setRequestProperty("User-Agent", "Mozilla/5.0");
            int length = connection.getContentLength();
            if (length > 5 * 1024 * 1024) return null;
            InputStream input = connection.getInputStream();
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            byte[] buffer = new byte[8192];
            int total = 0;
            int count;
            while ((count = input.read(buffer)) != -1) {
                total += count;
                if (total > 5 * 1024 * 1024) return null;
                output.write(buffer, 0, count);
            }
            input.close();
            BitmapFactory.Options options = new BitmapFactory.Options();
            options.inSampleSize = 2;
            return BitmapFactory.decodeByteArray(output.toByteArray(), 0, total, options);
        } catch (Exception ignored) {
            return null;
        } finally {
            if (connection != null) connection.disconnect();
        }
    }

    private static String secureOrigin(String url) {
        if (url == null) {
            return null;
        }
        Uri uri = Uri.parse(url);
        String scheme = uri.getScheme();
        String host = uri.getHost();
        if (!"https".equalsIgnoreCase(scheme) || host == null) {
            return null;
        }
        int port = uri.getPort();
        return "https://" + host.toLowerCase()
                + (port == -1 || port == 443 ? "" : ":" + port);
    }

    private boolean isAllowedNotificationOrigin() {
        return notificationOrigin != null
                && notificationOrigin.equals(currentNotificationOrigin);
    }

    private boolean notificationsAllowed() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) {
            return false;
        }
        NotificationManager manager = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        return manager != null && manager.areNotificationsEnabled();
    }

    private String notificationPermissionState() {
        if (notificationsAllowed()) {
            return "granted";
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                == PackageManager.PERMISSION_DENIED
                && !getPreferences(MODE_PRIVATE).getBoolean("web_notifications_requested", false)) {
            return "default";
        }
        return "denied";
    }

    private void requestNotificationPermission(int requestId) {
        if (!isAllowedNotificationOrigin()) {
            resolveNotificationPermission(requestId, false);
            return;
        }
        if (notificationsAllowed()) {
            resolveNotificationPermission(requestId, true);
            return;
        }
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            resolveNotificationPermission(requestId, false);
            return;
        }

        pendingNotificationRequests.add(requestId);
        if (!notificationPermissionRequestInFlight) {
            notificationPermissionRequestInFlight = true;
            getPreferences(MODE_PRIVATE).edit()
                    .putBoolean("web_notifications_requested", true)
                    .apply();
            requestPermissions(
                    new String[]{Manifest.permission.POST_NOTIFICATIONS},
                    NOTIFICATION_PERMISSION_REQUEST);
        }
    }

    private void resolveNotificationPermission(int requestId, boolean granted) {
        WebView webView = notificationWebView;
        if (webView != null) {
            webView.evaluateJavascript(
                    "window.__voidResolveNotificationPermission&&window.__voidResolveNotificationPermission("
                            + requestId + "," + granted + ")",
                    null);
        }
    }

    private void showWebNotification(String title, String body) {
        if (!isAllowedNotificationOrigin() || !notificationsAllowed()) {
            return;
        }
        NotificationManager manager = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (manager == null) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(new NotificationChannel(
                    NOTIFICATION_CHANNEL_ID, "Website notifications",
                    NotificationManager.IMPORTANCE_DEFAULT));
        }
        Notification.Builder builder = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O
                ? new Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
                : new Notification.Builder(this);
        builder.setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle(title.isEmpty() ? "Website notification" : title)
                .setContentText(body)
                .setAutoCancel(true);
        manager.notify(nextNotificationId.getAndIncrement(), builder.build());
    }

    @Override
    protected void onDestroy() {
        if (mediaSession != null) {
            mediaSession.setActive(false);
            mediaSession.release();
            mediaSession = null;
        }
        mediaWebView = null;
        if (artworkExecutor != null) artworkExecutor.shutdownNow();
        if (mediaActionReceiverRegistered) {
            unregisterReceiver(mediaActionReceiver);
            mediaActionReceiverRegistered = false;
        }
        super.onDestroy();
    }

    private final class VoidNotificationBridge {
        private final String bridgeToken;

        VoidNotificationBridge(String bridgeToken) {
            this.bridgeToken = bridgeToken;
        }

        private boolean isAuthorized(String token) {
            return bridgeToken.equals(token) && isAllowedNotificationOrigin();
        }

        @JavascriptInterface
        public String permission(String token) {
            if (!isAuthorized(token)) {
                return "denied";
            }
            return notificationPermissionState();
        }

        @JavascriptInterface
        public void requestPermission(final int requestId, String token) {
            if (!isAuthorized(token)) {
                return;
            }
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    requestNotificationPermission(requestId);
                }
            });
        }

        @JavascriptInterface
        public void show(final String title, final String body, String token) {
            if (!isAuthorized(token)) {
                return;
            }
            runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    showWebNotification(title == null ? "" : title,
                            body == null ? "" : body);
                }
            });
        }
    }

    private void requestMediaPermission(PermissionRequest request) {
        String[] resources = request.getResources();
        if (resources == null || resources.length == 0) {
            request.deny();
            return;
        }

        Set<String> runtimePermissions = new LinkedHashSet<>();
        for (String resource : resources) {
            if (PermissionRequest.RESOURCE_VIDEO_CAPTURE.equals(resource)) {
                runtimePermissions.add(Manifest.permission.CAMERA);
            } else if (PermissionRequest.RESOURCE_AUDIO_CAPTURE.equals(resource)) {
                runtimePermissions.add(Manifest.permission.RECORD_AUDIO);
            } else {
                request.deny();
                return;
            }
        }

        ArrayList<String> missing = new ArrayList<>();
        for (String permission : runtimePermissions) {
            if (checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
                missing.add(permission);
            }
        }

        if (missing.isEmpty()) {
            request.grant(resources);
            return;
        }

        if (pendingMediaRequest != null) {
            pendingMediaRequest.deny();
        }
        pendingMediaRequest = request;
        pendingMediaResources = resources;
        pendingMediaPermissions = missing.toArray(new String[0]);
        requestPermissions(pendingMediaPermissions, MEDIA_PERMISSION_REQUEST);
    }

    private void requestDownload(String url, String userAgent, String contentDisposition,
            String mimeType, String referer) {
        Uri uri = Uri.parse(url);
        if (!"http".equalsIgnoreCase(uri.getScheme())
                && !"https".equalsIgnoreCase(uri.getScheme())) {
            Toast.makeText(this, "This download link is not supported", Toast.LENGTH_LONG).show();
            return;
        }

        DownloadInfo info = new DownloadInfo(url, userAgent, contentDisposition, mimeType, referer);
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q
                && checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE)
                != PackageManager.PERMISSION_GRANTED) {
            pendingDownload = info;
            requestPermissions(
                    new String[]{Manifest.permission.WRITE_EXTERNAL_STORAGE},
                    STORAGE_PERMISSION_REQUEST);
            return;
        }
        enqueueDownload(info);
    }

    private void enqueueDownload(DownloadInfo info) {
        try {
            String filename = URLUtil.guessFileName(
                    info.url, info.contentDisposition, info.mimeType);
            DownloadManager.Request request = new DownloadManager.Request(Uri.parse(info.url));
            request.setTitle(filename);
            request.setNotificationVisibility(
                    DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED);
            request.setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, filename);
            request.setAllowedOverRoaming(false);

            if (info.mimeType != null && !info.mimeType.isEmpty()) {
                request.setMimeType(info.mimeType);
            }
            if (info.userAgent != null && !info.userAgent.isEmpty()) {
                request.addRequestHeader("User-Agent", info.userAgent);
            }
            String cookies = CookieManager.getInstance().getCookie(info.url);
            if (cookies != null && !cookies.isEmpty()) {
                request.addRequestHeader("Cookie", cookies);
            }
            if (info.referer != null && !info.referer.isEmpty()) {
                request.addRequestHeader("Referer", info.referer);
            }

            DownloadManager manager = (DownloadManager) getSystemService(DOWNLOAD_SERVICE);
            if (manager == null) {
                throw new IllegalStateException("Android DownloadManager is unavailable");
            }
            manager.enqueue(request);
            Toast.makeText(this, "Download started: " + filename, Toast.LENGTH_SHORT).show();
        } catch (RuntimeException error) {
            Toast.makeText(this, "Could not start download", Toast.LENGTH_LONG).show();
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions,
            int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);

        if (requestCode == MEDIA_PERMISSION_REQUEST) {
            PermissionRequest request = pendingMediaRequest;
            String[] resources = pendingMediaResources;
            String[] requiredPermissions = pendingMediaPermissions;
            clearPendingMediaRequest();
            if (request == null || resources == null || requiredPermissions == null) {
                return;
            }

            boolean granted = true;
            for (String permission : requiredPermissions) {
                if (checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
                    granted = false;
                    break;
                }
            }
            if (granted) {
                request.grant(resources);
            } else {
                request.deny();
            }
        } else if (requestCode == STORAGE_PERMISSION_REQUEST) {
            DownloadInfo download = pendingDownload;
            pendingDownload = null;
            if (download == null) {
                return;
            }
            if (grantResults.length > 0
                    && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
                enqueueDownload(download);
            } else {
                Toast.makeText(this, "Storage permission is required for downloads",
                        Toast.LENGTH_LONG).show();
            }
        } else if (requestCode == NOTIFICATION_PERMISSION_REQUEST) {
            notificationPermissionRequestInFlight = false;
            boolean granted = notificationsAllowed();
            ArrayList<Integer> requestIds = new ArrayList<>(pendingNotificationRequests);
            pendingNotificationRequests.clear();
            for (Integer requestId : requestIds) {
                resolveNotificationPermission(requestId, granted);
            }
        }
    }

    private void clearPendingMediaRequest() {
        pendingMediaRequest = null;
        pendingMediaResources = null;
        pendingMediaPermissions = null;
    }

    private static final class DownloadInfo {
        final String url;
        final String userAgent;
        final String contentDisposition;
        final String mimeType;
        final String referer;

        DownloadInfo(String url, String userAgent, String contentDisposition,
                String mimeType, String referer) {
            this.url = url;
            this.userAgent = userAgent;
            this.contentDisposition = contentDisposition;
            this.mimeType = mimeType;
            this.referer = referer;
        }
    }
}
