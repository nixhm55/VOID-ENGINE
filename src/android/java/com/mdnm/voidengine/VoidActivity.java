package com.mdnm.voidengine;

import android.Manifest;
import android.app.DownloadManager;
import android.app.NativeActivity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
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

import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicInteger;

public final class VoidActivity extends NativeActivity {
    private static final int MEDIA_PERMISSION_REQUEST = 0x564f;
    private static final int STORAGE_PERMISSION_REQUEST = 0x564e;
    private static final int NOTIFICATION_PERMISSION_REQUEST = 0x564d;
    private static final String NOTIFICATION_CHANNEL_ID = "void_web_notifications";
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

    private PermissionRequest pendingMediaRequest;
    private String[] pendingMediaResources;
    private String[] pendingMediaPermissions;
    private DownloadInfo pendingDownload;
    private WebView notificationWebView;
    private String notificationOrigin;
    private String notificationBridgeToken;
    private volatile String currentNotificationOrigin;
    private boolean notificationPermissionRequestInFlight;
    private final ArrayList<Integer> pendingNotificationRequests = new ArrayList<>();
    private final AtomicInteger nextNotificationId = new AtomicInteger(1);

    public void configureVoidWebView(final WebView webView, String allowedUrl) {
        notificationWebView = webView;
        notificationOrigin = secureOrigin(allowedUrl);
        notificationBridgeToken = UUID.randomUUID().toString();
        webView.addJavascriptInterface(
                new VoidNotificationBridge(notificationBridgeToken), "VoidNativeNotifications");
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
    }

    private void injectNotificationShim(WebView webView, String url) {
        currentNotificationOrigin = secureOrigin(url);
        if (notificationOrigin != null && notificationOrigin.equals(currentNotificationOrigin)) {
            webView.evaluateJavascript(
                    NOTIFICATION_SHIM.replace("__VOID_TOKEN__", notificationBridgeToken), null);
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
