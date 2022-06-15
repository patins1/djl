/*
 * Copyright 2021 Amazon.com, Inc. or its affiliates. All Rights Reserved.
 *
 * Licensed under the Apache License, Version 2.0 (the "License"). You may not use this file except in compliance
 * with the License. A copy of the License is located at
 *
 * http://aws.amazon.com/apache2.0/
 *
 * or in the "license" file accompanying this file. This file is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES
 * OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific language governing permissions
 * and limitations under the License.
 */
package ai.djl.opencv;

import ai.djl.modality.cv.Image;
import ai.djl.modality.cv.output.BoundingBox;
import ai.djl.modality.cv.output.DetectedObjects;
import ai.djl.modality.cv.output.Joints;
import ai.djl.modality.cv.output.Landmark;
import ai.djl.modality.cv.output.Mask;
import ai.djl.modality.cv.output.Rectangle;
import ai.djl.ndarray.NDArray;
import ai.djl.ndarray.NDManager;
import ai.djl.ndarray.types.DataType;
import ai.djl.ndarray.types.Shape;
import ai.djl.util.RandomUtils;
import java.awt.Color;
import java.awt.Graphics2D;
import java.awt.image.BufferedImage;
import java.awt.image.DataBufferByte;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;
import org.opencv.core.CvType;
import org.opencv.core.Mat;
import org.opencv.core.MatOfByte;
import org.opencv.core.MatOfPoint;
import org.opencv.core.Point;
import org.opencv.core.Rect;
import org.opencv.core.Scalar;
import org.opencv.core.Size;
import org.opencv.imgcodecs.Imgcodecs;
import org.opencv.imgproc.Imgproc;

/** {@code OpenCVImage} is a high performance implementation of {@link Image}. */
class OpenCVImage implements Image {

    private Mat image;

    /**
     * Constructs a new {@code OpenCVImage} instance.
     *
     * @param image the wrapped image
     */
    public OpenCVImage(Mat image) {
        this.image = image;
    }

    /** {@inheritDoc} */
    @Override
    public int getWidth() {
        return image.width();
    }

    /** {@inheritDoc} */
    @Override
    public int getHeight() {
        return image.height();
    }

    /** {@inheritDoc} */
    @Override
    public Object getWrappedImage() {
        return image;
    }

    /** {@inheritDoc} */
    @Override
    public Image getSubImage(int x, int y, int w, int h) {
        Mat mat = image.submat(new Rect(x, y, w, h));
        return new OpenCVImage(mat);
    }

    /** {@inheritDoc} */
    @Override
    public Image duplicate() {
        Mat mat = new Mat();
        image.copyTo(mat);
        return new OpenCVImage(mat);
    }

    /** {@inheritDoc} */
    @Override
    public NDArray toNDArray(NDManager manager, Flag flag) {
        Mat mat = new Mat();
        if (flag == Flag.GRAYSCALE) {
            Imgproc.cvtColor(image, mat, Imgproc.COLOR_BGR2GRAY);
        } else {
            Imgproc.cvtColor(image, mat, Imgproc.COLOR_BGR2RGB);
        }
        byte[] buf = new byte[mat.height() * mat.width() * mat.channels()];
        mat.get(0, 0, buf);

        Shape shape = new Shape(mat.height(), mat.width(), mat.channels());
        return manager.create(ByteBuffer.wrap(buf), shape, DataType.UINT8);
    }

    /** {@inheritDoc} */
    @Override
    public void save(OutputStream os, String type) throws IOException {
        MatOfByte buf = new MatOfByte();
        if (!Imgcodecs.imencode('.' + type, image, buf)) {
            throw new IOException("Failed save image.");
        }
        os.write(buf.toArray());
    }

    /** {@inheritDoc} */
    @Override
    public void drawBoundingBoxes(DetectedObjects detections) {
        int imageWidth = image.width();
        int imageHeight = image.height();

        List<DetectedObjects.DetectedObject> list = detections.items();
        for (DetectedObjects.DetectedObject result : list) {
            String className = result.getClassName();
            BoundingBox box = result.getBoundingBox();

            Rectangle rectangle = box.getBounds();
            int x = (int) (rectangle.getX() * imageWidth);
            int y = (int) (rectangle.getY() * imageHeight);
            Rect rect =
                    new Rect(
                            x,
                            y,
                            (int) (rectangle.getWidth() * imageWidth),
                            (int) (rectangle.getHeight() * imageHeight));
            Scalar color =
                    new Scalar(
                            RandomUtils.nextInt(178),
                            RandomUtils.nextInt(178),
                            RandomUtils.nextInt(178));
            Imgproc.rectangle(image, rect.tl(), rect.br(), color, 2);

            Size size = Imgproc.getTextSize(className, Imgproc.FONT_HERSHEY_PLAIN, 1.3, 1, null);
            Point br = new Point(x + size.width + 4, y + size.height + 4);
            Imgproc.rectangle(image, rect.tl(), br, color, -1);

            Point point = new Point(x, y + size.height + 2);
            color = new Scalar(255, 255, 255);
            Imgproc.putText(image, className, point, Imgproc.FONT_HERSHEY_PLAIN, 1.3, color, 1);
            // If we have a mask instead of a plain rectangle, draw tha mask
            if (box instanceof Mask) {
                Mask mask = (Mask) box;
                BufferedImage img = mat2Image(image);
                drawMask(img, mask);
                image = image2Mat(img);
            } else if (box instanceof Landmark) {
                drawLandmarks(box);
            }
        }
    }

    /** {@inheritDoc} */
    @Override
    public void drawJoints(Joints joints) {
        int imageWidth = image.width();
        int imageHeight = image.height();

        Scalar color =
                new Scalar(
                        RandomUtils.nextInt(178),
                        RandomUtils.nextInt(178),
                        RandomUtils.nextInt(178));
        for (Joints.Joint joint : joints.getJoints()) {
            int x = (int) (joint.getX() * imageWidth);
            int y = (int) (joint.getY() * imageHeight);
            Point point = new Point(x, y);
            Imgproc.circle(image, point, 6, color, -1, Imgproc.LINE_AA);
        }
    }

    /** {@inheritDoc} */
    @Override
    public List<BoundingBox> findBoundingBoxes() {
        List<MatOfPoint> points = new ArrayList<>();
        Imgproc.findContours(
                image, points, new Mat(), Imgproc.RETR_LIST, Imgproc.CHAIN_APPROX_SIMPLE);
        return points.parallelStream()
                .map(
                        point -> {
                            Rect rect = Imgproc.boundingRect(point);
                            return new Rectangle(
                                    rect.x * 1.0 / image.width(),
                                    rect.y * 1.0 / image.height(),
                                    rect.width * 1.0 / image.width(),
                                    rect.height * 1.0 / image.height());
                        })
                .collect(Collectors.toList());
    }

    private void drawLandmarks(BoundingBox box) {
        Scalar color = new Scalar(0, 96, 246);
        for (ai.djl.modality.cv.output.Point point : box.getPath()) {
            Point lt = new Point(point.getX() - 4, point.getY() - 4);
            Point rb = new Point(point.getX() + 4, point.getY() + 4);
            Imgproc.rectangle(image, lt, rb, color, -1);
        }
    }

    private void drawMask(BufferedImage img, Mask mask) {
        // TODO: use OpenCV native way to draw mask
        float r = RandomUtils.nextFloat();
        float g = RandomUtils.nextFloat();
        float b = RandomUtils.nextFloat();
        int imageWidth = img.getWidth();
        int imageHeight = img.getHeight();
        int x = (int) (mask.getX() * imageWidth);
        int y = (int) (mask.getY() * imageHeight);
        float[][] probDist = mask.getProbDist();
        // Correct some coordinates of box when going out of image
        if (x < 0) {
            x = 0;
        }
        if (y < 0) {
            y = 0;
        }

        BufferedImage maskImage =
                new BufferedImage(probDist.length, probDist[0].length, BufferedImage.TYPE_INT_ARGB);
        for (int xCor = 0; xCor < probDist.length; xCor++) {
            for (int yCor = 0; yCor < probDist[xCor].length; yCor++) {
                float opacity = probDist[xCor][yCor] * 0.8f;
                maskImage.setRGB(xCor, yCor, new Color(r, g, b, opacity).getRGB());
            }
        }
        Graphics2D gR = (Graphics2D) img.getGraphics();
        gR.drawImage(maskImage, x, y, null);
        gR.dispose();
    }

    private static BufferedImage mat2Image(Mat mat) {
        int width = mat.width();
        int height = mat.height();
        byte[] data = new byte[width * height * (int) mat.elemSize()];
        Imgproc.cvtColor(mat, mat, Imgproc.COLOR_BGR2RGB);

        mat.get(0, 0, data);

        BufferedImage ret = new BufferedImage(width, height, BufferedImage.TYPE_3BYTE_BGR);
        ret.getRaster().setDataElements(0, 0, width, height, data);
        return ret;
    }

    private static Mat image2Mat(BufferedImage img) {
        int width = img.getWidth();
        int height = img.getHeight();
        byte[] data = ((DataBufferByte) img.getRaster().getDataBuffer()).getData();
        Mat mat = new Mat(height, width, CvType.CV_8UC3);
        mat.put(0, 0, data);
        return mat;
    }
}
