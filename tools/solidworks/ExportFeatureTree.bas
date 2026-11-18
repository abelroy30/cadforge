Attribute VB_Name = "ExportFeatureTree"
'=====================================================================
' ExportFeatureTree - write a SolidWorks feature tree out as JSON.
'
' WHY THIS EXISTS
'   The real modelling history of a part lives inside the .SLDPRT/.SLDASM
'   binary, in a proprietary format. Rather than reverse-engineer it,
'   this macro asks SolidWorks for the tree it already knows and writes
'   it to a small JSON file. cadforge reads that file as the true
'   history of the part - better than anything inferred from geometry.
'
' HOW TO INSTALL
'   1. In SolidWorks: Tools > Macro > New, save as ExportFeatureTree.swp.
'      (Or paste this into the VBA editor: Tools > Macro > Edit.)
'   2. Open the part or assembly you want to export.
'   3. Run the macro (Tools > Macro > Run).
'   4. It writes <DocumentName>.features.json next to the model file.
'      Upload that JSON to cadforge - it becomes a History artifact.
'
' OPTIONAL BULK RUN
'   Set SWBATCH_ROOT below to a folder and the macro will walk every
'   .SLDPRT/.SLDASM under it, exporting each tree. Leave it empty to
'   export only the open document.
'=====================================================================
Option Explicit

' Leave empty to export just the open document.
Private Const SWBATCH_ROOT As String = ""

'---------------------------------------------------------------- model

Public Sub main()

    Dim swApp As SldWorks.SldWorks
    Set swApp = Application.SldWorks

    If Len(SWBATCH_ROOT) > 0 Then
        ExportFolder swApp, SWBATCH_ROOT
        MsgBox "Exported every model under " & vbNewLine & SWBATCH_ROOT, vbInformation
        Exit Sub
    End If

    Dim doc As SldWorks.ModelDoc2
    Set doc = swApp.ActiveDoc
    If doc Is Nothing Then
        MsgBox "Open a part or assembly first.", vbExclamation
        Exit Sub
    End If

    Dim outPath As String
    outPath = OutPathFor(doc)
    WriteExport doc, outPath
    MsgBox "Wrote " & vbNewLine & outPath, vbInformation
End Sub

'---------------------------------------------------------------- batch

Private Sub ExportFolder(swApp As SldWorks.SldWorks, root As String)

    Dim fso As Object
    Set fso = CreateObject("Scripting.FileSystemObject")
    If Not fso.FolderExists(root) Then
        MsgBox "No such folder: " & root, vbExclamation
        Exit Sub
    End If

    Dim folder As Object, file As Object, sub_ As Object
    Dim ext As String, doc As SldWorks.ModelDoc2

    Set folder = fso.GetFolder(root)
    For Each file In folder.Files
        ext = LCase$(fso.GetExtensionName(file.Name))
        If ext = "sldprt" Or ext = "sldasm" Then
            Set doc = swApp.OpenDoc6(file.Path, DocTypeFor(ext), 1, "", 0, 0)
            If Not doc Is Nothing Then
                WriteExport doc, fso.BuildPath(root, fso.GetBaseName(file.Name) & ".features.json")
                swApp.CloseDoc doc.GetTitle
            End If
        End If
    Next file

    ' Recurse into subfolders.
    For Each sub_ In folder.SubFolders
        ExportFolder swApp, sub_.Path
    Next sub_
End Sub

Private Function DocTypeFor(ext As String) As Long
    ' swDocPART = 1, swDocASSEMBLY = 2
    If ext = "sldasm" Then DocTypeFor = 2 Else DocTypeFor = 1
End Function

Private Function OutPathFor(doc As SldWorks.ModelDoc2) As String
    Dim p As String
    p = doc.GetPathName
    If Len(p) = 0 Then
        OutPathFor = Environ$("USERPROFILE") & "\" & doc.GetTitle & ".features.json"
    Else
        OutPathFor = Left$(p, InStrRev(p, ".")) & "features.json"
    End If
End Function

'---------------------------------------------------------------- export

Private Sub WriteExport(doc As SldWorks.ModelDoc2, ByVal outPath As String)

    Dim sb As String
    sb = "{" & vbLf
    sb = sb & "  " & Q("source") & ": " & Q("SolidWorks " & SwVersionText()) & "," & vbLf
    sb = sb & "  " & Q("document") & ": " & Q(FileNameOf(doc)) & "," & vbLf
    sb = sb & "  " & Q("configuration") & ": " & Q(ActiveConfigName(doc)) & "," & vbLf
    sb = sb & "  " & Q("features") & ": [" & vbLf

    Dim feat As SldWorks.Feature
    Set feat = doc.FirstFeature

    Dim first As Boolean
    first = True

    Do While Not feat Is Nothing
        sb = sb & FeatureJson(feat, first)
        first = False
        Set feat = feat.GetNextFeature
    Loop

    ' Close the array (drop a trailing comma if one was left).
    sb = RemoveTrailingComma(sb)
    sb = sb & vbLf & "  ]" & vbLf & "}" & vbLf

    Dim fnum As Integer
    fnum = FreeFile
    Open outPath For Output As #fnum
    Print #fnum, sb
    Close #fnum
End Sub

' One feature, and its children. Folders and assemblies keep their children
' indented under them, so the tree we write mirrors what the user sees.
Private Function FeatureJson(feat As SldWorks.Feature, first As Boolean) As String

    Dim s As String
    If Not first Then s = "," & vbLf

    s = s & "    {" & vbLf
    s = s & "      " & Q("name") & ": " & Q(feat.Name) & "," & vbLf
    s = s & "      " & Q("type") & ": " & Q(feat.GetTypeName2) & "," & vbLf

    If feat.IsSuppressed Then
        s = s & "      " & Q("suppressed") & ": true," & vbLf
    End If

    s = s & "      " & Q("details") & ": " & DetailsJson(feat) & vbLf
    s = s & "    }"

    FeatureJson = s
End Function

' The dimensions SolidWorks exposes for a feature, as a flat object.
' Numeric values are written as numbers so the reader sees 12, not "12".
Private Function DetailsJson(feat As SldWorks.Feature) As String

    Dim s As String
    s = "{"

    Dim disp As SldWorks.DisplayDimension
    Set disp = feat.GetFirstDisplayDimension

    Dim first As Boolean
    first = True
    Dim n As Integer
    n = 0

    Do While Not disp Is Nothing
        Dim dim_ As SldWorks.Dimension
        Set dim_ = disp.GetDimension2(0)
        If Not dim_ Is Nothing Then
            Dim key As String, val As String
            key = dim_.FullName
            ' Keep the readable tail of a long name like D1@Sketch1.
            If InStr(key, "@") > 1 Then key = Left$(key, InStr(key, "@") - 1)
            key = "dim" & CStr(n + 1)

            s = s & ", " & Q(key) & ": " & Q(dim_.Value & " " & dim_.GetUnitsString)
            n = n + 1
            first = False
        End If
        Set disp = feat.GetNextDisplayDimension(disp)
        ' Guard against a pathological loop on an odd feature.
        If n > 200 Then Exit Do
    Loop

    If feat.IsSuppressed Then
        s = s & ", " & Q("suppressed") & ": true"
        first = False
    End If

    DetailsJson = s & " }"
End Function

'---------------------------------------------------------------- helpers

' A JSON string in double quotes, with the characters JSON cares about escaped.
Private Function Q(ByVal s As String) As String
    s = Replace(s, "\", "\\")
    s = Replace(s, """", "\""")
    s = Replace(s, vbCr, " ")
    s = Replace(s, vbLf, " ")
    s = Replace(s, vbTab, " ")
    Q = """" & s & """"
End Function

Private Function RemoveTrailingComma(ByVal s As String) As String
    Dim t As String
    t = RTrim$(s)
    If Right$(t, 1) = "," Then t = Left$(t, Len(t) - 1)
    RemoveTrailingComma = t
End Function

Private Function FileNameOf(doc As SldWorks.ModelDoc2) As String
    Dim p As String
    p = doc.GetPathName
    If Len(p) = 0 Then
        FileNameOf = doc.GetTitle
    Else
        FileNameOf = Mid$(p, InStrRev(p, "\") + 1)
    End If
End Function

Private Function ActiveConfigName(doc As SldWorks.ModelDoc2) As String
    Dim cfg As SldWorks.Configuration
    Set cfg = doc.GetActiveConfiguration
    If cfg Is Nothing Then ActiveConfigName = "" Else ActiveConfigName = cfg.Name
End Function

Private Function SwVersionText() As String
    Dim swApp As SldWorks.SldWorks
    Set swApp = Application.SldWorks
    SwVersionText = swApp.RevisionNumber
End Function
